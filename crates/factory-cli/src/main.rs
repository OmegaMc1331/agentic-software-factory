use std::path::Path;
use std::process::Command as ProcessCommand;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use factory_core::{Agents, Config, Factory, FACTORY_DIR};
use factory_db::FactoryDb;
use factory_types::TaskState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    cli.run()
}

#[derive(Parser)]
#[command(
    name = "factory",
    version,
    about = "Agentic Software Factory: coordinate coding agents through structured tasks and isolated git worktrees."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the factory state in the current directory
    Init,
    /// Start the local factory application (API + dashboard)
    Start {
        /// Port to bind (default: 4321)
        #[arg(long, default_value_t = 4321)]
        port: u16,
        /// Do not open the dashboard in a browser
        #[arg(long)]
        no_browser: bool,
    },
    /// Plan a run from a software objective using the configured planner agent
    Run {
        /// The objective to plan
        objective: String,
    },
    /// Show the current factory status
    Status,
    /// Internal and development commands
    #[command(subcommand)]
    Dev(DevCommand),
}

#[derive(Subcommand)]
enum DevCommand {
    /// List configured agents and whether their executable is on PATH
    Agents,
    /// Inspect the agent configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// List tasks of a run (default: the latest run)
    Tasks {
        /// Run id to list
        #[arg(long)]
        run: Option<i64>,
    },
    /// Show details of a single task
    Inspect {
        /// Task id
        task: i64,
    },
    /// Move a task to a new state
    Mark {
        /// Task id
        task: i64,
        /// New state: pending, ready, running, blocked, failed, completed
        state: String,
    },
    /// Manage isolated git worktrees for tasks
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
    /// Serve the local HTTP API for the dashboard
    Serve {
        /// Port to bind (default: 4321)
        #[arg(long, default_value_t = 4321)]
        port: u16,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Show the role-to-agent mapping
    List,
}

#[derive(Subcommand)]
enum WorktreeCommand {
    /// Create a worktree for a ready task
    Create {
        /// Task id
        task: i64,
    },
    /// Remove the worktree of a task (refuses a dirty worktree unless --force)
    Remove {
        /// Task id
        task: i64,
        /// Remove even with uncommitted changes
        #[arg(long)]
        force: bool,
    },
    /// List worktrees of the repository
    Status,
}

impl Cli {
    fn run(&self) -> Result<()> {
        let root = std::env::current_dir().context("cannot resolve current directory")?;
        match &self.command {
            Command::Init => {
                init(&root)?;
            }
            Command::Start { port, no_browser } => {
                start(&root, *port, *no_browser)?;
            }
            Command::Run { objective } => {
                run(&root, objective)?;
            }
            Command::Status => {
                status(&root)?;
            }
            Command::Dev(command) => match command {
                DevCommand::Agents => agents(&root)?,
                DevCommand::Config { command } => match command {
                    ConfigCommand::List => config_list(&root)?,
                },
                DevCommand::Tasks { run } => tasks(&root, *run)?,
                DevCommand::Inspect { task } => inspect(&root, *task)?,
                DevCommand::Mark { task, state } => mark(&root, *task, state)?,
                DevCommand::Worktree { command } => match command {
                    WorktreeCommand::Create { task } => worktree_create(&root, *task)?,
                    WorktreeCommand::Remove { task, force } => {
                        worktree_remove(&root, *task, *force)?
                    }
                    WorktreeCommand::Status => worktree_status(&root)?,
                },
                DevCommand::Serve { port } => serve(&root, *port)?,
            },
        }
        Ok(())
    }
}

fn factory_root(root: &Path) -> Result<()> {
    if !root.join(FACTORY_DIR).join("db.sqlite3").exists() {
        bail!("no factory state found here; run `factory init` first");
    }
    Ok(())
}

fn init(root: &Path) -> Result<()> {
    let factory_dir = root.join(FACTORY_DIR);
    let already = factory_dir.join("db.sqlite3").exists();
    Factory::init(root)?;
    if already {
        println!("Factory already initialized.");
        return Ok(());
    }
    println!("Initialized factory state at {}", factory_dir.display());
    println!("Database: {}", factory_dir.join("db.sqlite3").display());
    println!(
        "Configuration: {}",
        factory_dir.join("config.toml").display()
    );
    println!(
        "Configure coding agents from the dashboard (`factory start`) or edit the file directly."
    );
    Ok(())
}

fn agents(root: &Path) -> Result<()> {
    let agents = Agents::load(root).context("run `factory init` to create config.toml first")?;
    let infos = agents.list();
    if infos.is_empty() {
        println!("no agents configured");
        return Ok(());
    }
    println!("{:<12} {:<20} {:<10}", "NAME", "COMMAND", "STATUS");
    for info in infos {
        let status = if info.available {
            "available"
        } else {
            "missing"
        };
        println!("{:<12} {:<20} {}", info.name, info.command, status);
    }
    Ok(())
}

fn config_list(root: &Path) -> Result<()> {
    let config = Config::load(root).context("run `factory init` to create config.toml first")?;
    if config.roles.is_empty() {
        println!("no roles configured");
        return Ok(());
    }
    for (role, entry) in &config.roles {
        println!("{:<12} {}", role, entry.agent);
    }
    Ok(())
}

fn run(root: &Path, objective: &str) -> Result<()> {
    factory_root(root)?;
    let factory = Factory::open(root)?;
    let outcome = factory.create_run(objective)?;
    println!(
        "Run #{} planned (planner: {}, {} tasks)",
        outcome.run.id,
        outcome.run.planner_agent.as_deref().unwrap_or("_"),
        outcome.tasks.len()
    );
    for task in &outcome.tasks {
        print_task(task);
    }
    println!();
    println!(
        "Inspect tasks with `factory dev tasks --run {}` or `factory dev inspect <task-id>`.",
        outcome.run.id
    );
    Ok(())
}

fn status(root: &Path) -> Result<()> {
    factory_root(root)?;
    let db = FactoryDb::open(&root.join(FACTORY_DIR).join("db.sqlite3"))?;
    let runs = db.list_runs()?;
    println!("Factory: {}", root.join(FACTORY_DIR).display());
    match runs.first() {
        None => {
            println!("No runs yet. Create one with `factory run \"<objective>\"`.");
        }
        Some(run) => {
            println!("Latest run: #{} ({})", run.id, run.status.as_str());
            print_run(&db, run)?;
            println!();
            for task in &db.list_tasks(run.id)? {
                print_task(task);
            }
        }
    }
    Ok(())
}

fn tasks(root: &Path, run_id: Option<i64>) -> Result<()> {
    factory_root(root)?;
    let db = FactoryDb::open(&root.join(FACTORY_DIR).join("db.sqlite3"))?;
    let run = match run_id {
        Some(id) => db.get_run(id)?.context(format!("run {id} not found"))?,
        None => db.list_runs()?.first().cloned().context("no runs found")?,
    };
    println!("Run #{} — {}", run.id, run.objective);
    for task in &db.list_tasks(run.id)? {
        let deps = deps_label(&task.dependencies);
        println!(
            "  {:>3}  {:<9} {}{}",
            task.id,
            task.state.as_str(),
            task.title,
            if deps.is_empty() {
                String::new()
            } else {
                format!(" [{}]", deps)
            },
        );
    }
    Ok(())
}

fn inspect(root: &Path, task_id: i64) -> Result<()> {
    factory_root(root)?;
    let db = FactoryDb::open(&root.join(FACTORY_DIR).join("db.sqlite3"))?;
    let task = db
        .get_task(task_id)?
        .context(format!("task {task_id} not found"))?;
    let deps = deps_label(&task.dependencies);
    println!("Task #{} (run {})", task.id, task.run_id);
    println!("State:         {}", task.state.as_str());
    println!("Position:      {}", task.position);
    println!("Title:         {}", task.title);
    println!("Objective:     {}", task.objective);
    println!(
        "Dependencies:  {}",
        if deps.is_empty() {
            "(none)".into()
        } else {
            deps
        }
    );
    println!(
        "Worktree:      {}",
        task.worktree_path.as_deref().unwrap_or("(none)")
    );
    println!("Created:       {}", task.created_at);
    println!("Updated:       {}", task.updated_at);
    println!("Acceptance criteria:");
    for criterion in &task.acceptance_criteria {
        println!("  - {}", criterion);
    }
    Ok(())
}

fn mark(root: &Path, task_id: i64, state: &str) -> Result<()> {
    factory_root(root)?;
    let target: TaskState = state
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid state '{state}': {e}"))?;
    let factory = Factory::open(root)?;
    let outcome = factory.mark_task(task_id, target)?;
    println!(
        "task #{}: {} -> {}",
        outcome.task.id,
        outcome.from.as_str(),
        target.as_str()
    );
    for id in outcome.updated.iter().skip(1) {
        println!("  propagated: task #{id} updated");
    }
    Ok(())
}

fn worktree_create(root: &Path, task_id: i64) -> Result<()> {
    factory_root(root)?;
    let factory = Factory::open(root)?;
    let path = factory.create_worktree(task_id)?;
    println!("created worktree at {}", path.display());
    Ok(())
}

fn worktree_remove(root: &Path, task_id: i64, force: bool) -> Result<()> {
    factory_root(root)?;
    let factory = Factory::open(root)?;
    factory.remove_worktree(task_id, force)?;
    if force {
        println!("removed worktree for task #{task_id} (--force)");
    } else {
        println!("removed worktree for task #{task_id}");
    }
    Ok(())
}

fn worktree_status(root: &Path) -> Result<()> {
    factory_root(root)?;
    let factory = Factory::open(root)?;
    let worktrees = factory
        .list_worktrees()
        .context("not inside a git repository")?;
    for w in worktrees {
        let branch = w.branch.as_deref().unwrap_or("(detached)");
        println!("  {:>3}  {}", w.path.display(), branch);
    }
    Ok(())
}

fn start(root: &Path, port: u16, no_browser: bool) -> Result<()> {
    factory_root(root)?;
    let db = FactoryDb::open(&root.join(FACTORY_DIR).join("db.sqlite3"))?;
    let state = factory_api::ApiState {
        db: Mutex::new(db),
        root: root.to_path_buf(),
    };
    let listener = factory_api::bind(port)?;
    let address = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}", address.port());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let mut server = tokio::spawn(factory_api::serve(Arc::new(state), listener));
        tokio::select! {
            result = &mut server => {
                result.context("Factory API task failed")??;
                bail!("Factory API stopped before becoming ready");
            }
            result = wait_until_ready(address) => result?,
        }

        println!("Agentic Software Factory running at {url}");
        if !no_browser {
            open_browser(&url);
        }

        server.await.context("Factory API task failed")??;
        Ok(())
    })
}

async fn wait_until_ready(address: std::net::SocketAddr) -> Result<()> {
    let response = tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = tokio::net::TcpStream::connect(address).await?;
        stream.set_nodelay(true)?;
        let request =
            format!("GET /api/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;
        let mut response = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..read]);
            if response
                .windows(br#"{"status":"ok"}"#.len())
                .any(|window| window == br#"{"status":"ok"}"#)
            {
                break;
            }
        }
        Ok::<Vec<u8>, std::io::Error>(response)
    })
    .await
    .context("Factory API did not become ready within 5 seconds")??;

    let response = String::from_utf8_lossy(&response);
    if !response.starts_with("HTTP/1.1 200 OK") || !response.contains(r#"{"status":"ok"}"#) {
        bail!("Factory API health check did not report a healthy status");
    }
    Ok(())
}

fn serve(root: &Path, port: u16) -> Result<()> {
    factory_root(root)?;
    let db = FactoryDb::open(&root.join(FACTORY_DIR).join("db.sqlite3"))?;
    let state = factory_api::ApiState {
        db: Mutex::new(db),
        root: root.to_path_buf(),
    };
    let listener = factory_api::bind(port)?;
    println!(
        "Factory API listening on http://127.0.0.1:{}",
        listener.local_addr()?.port()
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(factory_api::serve(Arc::new(state), listener))?;
    Ok(())
}

fn open_browser(url: &str) {
    let result = if cfg!(windows) {
        ProcessCommand::new("cmd")
            .args(["/C", "start", ""])
            .arg(url)
            .status()
    } else if cfg!(target_os = "macos") {
        ProcessCommand::new("open").arg(url).status()
    } else {
        ProcessCommand::new("xdg-open").arg(url).status()
    };
    if let Err(err) = result {
        eprintln!("could not open the browser: {err}");
    }
}

fn print_run(db: &FactoryDb, run: &factory_types::Run) -> Result<()> {
    let tasks = db.list_tasks(run.id)?;
    let counts = factory_api::types::TaskCounts::from_tasks(&tasks);
    println!(
        "  created {}  planner {}",
        run.created_at,
        run.planner_agent.as_deref().unwrap_or("_")
    );
    println!(
        "  tasks: {} pending, {} ready, {} running, {} blocked, {} failed, {} completed",
        counts.pending,
        counts.ready,
        counts.running,
        counts.blocked,
        counts.failed,
        counts.completed
    );
    Ok(())
}

fn deps_label(deps: &[i64]) -> String {
    deps.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_task(task: &factory_types::Task) {
    let deps = deps_label(&task.dependencies);
    println!(
        "  #{:<4} {:<9} {}{}",
        task.id,
        task.state.as_str(),
        task.title,
        if deps.is_empty() {
            String::new()
        } else {
            format!(" [{}]", deps)
        },
    );
}
