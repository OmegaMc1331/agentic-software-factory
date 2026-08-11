use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use factory_core::provider::LocalProvider;
use factory_core::{config_from_env, provider::build_provider, FACTORY_DIR};
use factory_core::{Factory, Provider};
use factory_db::FactoryDb;
use factory_models::TaskState;

fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    cli.run()
}

#[derive(Parser)]
#[command(
    name = "factory",
    version,
    about = "Agentic Software Factory: orchestrate coding agents across structured tasks, isolated git worktrees and verifiable execution."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize the factory state in the current directory
    Init {
        /// Re-initialize, overwriting existing state
        #[arg(long)]
        force: bool,
    },
    /// Plan a run from a software objective
    Run {
        /// The objective to plan
        objective: String,
    },
    /// Show the current factory status
    Status,
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
enum WorktreeCommand {
    /// Create a worktree for a ready task
    Create {
        /// Task id
        task: i64,
    },
    /// Remove the worktree of a task
    Remove {
        /// Task id
        task: i64,
    },
    /// List worktrees of the repository
    Status,
}

impl Cli {
    fn run(&self) -> Result<()> {
        let root = std::env::current_dir().context("cannot resolve current directory")?;
        match &self.command {
            Command::Init { force } => {
                init(&root, *force)?;
            }
            Command::Run { objective } => {
                run(&root, objective)?;
            }
            Command::Status => {
                status(&root)?;
            }
            Command::Tasks { run } => {
                tasks(&root, *run)?;
            }
            Command::Inspect { task } => {
                inspect(&root, *task)?;
            }
            Command::Mark { task, state } => {
                mark(&root, *task, state)?;
            }
            Command::Worktree { command } => match command {
                WorktreeCommand::Create { task } => worktree_create(&root, *task)?,
                WorktreeCommand::Remove { task } => worktree_remove(&root, *task)?,
                WorktreeCommand::Status => worktree_status(&root)?,
            },
            Command::Serve { port } => {
                serve(&root, *port)?;
            }
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

fn fallback_provider() -> Box<dyn Provider> {
    Box::new(LocalProvider::new())
}

fn init(root: &Path, force: bool) -> Result<()> {
    let cfg = config_from_env();
    let provider = build_provider(&cfg).unwrap_or_else(|_| fallback_provider());
    let factory = Factory::init(root, force, provider)?;
    println!(
        "Initialized factory state at {}",
        root.join(FACTORY_DIR).display()
    );
    println!(
        "Database: {}",
        root.join(FACTORY_DIR).join("db.sqlite3").display()
    );
    println!("Provider: {}", factory.provider());
    Ok(())
}

fn run(root: &Path, objective: &str) -> Result<()> {
    factory_root(root)?;
    let cfg = config_from_env();
    let provider = build_provider(&cfg).context(
        "cannot configure model provider; set FACTORY_API_KEY or use FACTORY_PROVIDER=local",
    )?;
    let factory = Factory::open(root, provider)?;
    let outcome = factory.create_run(objective)?;
    println!(
        "Run #{} planned ({}, {} tasks)",
        outcome.run.id,
        outcome.run.model.as_deref().unwrap_or("_"),
        outcome.tasks.len()
    );
    for task in &outcome.tasks {
        print_task(task);
    }
    println!();
    println!(
        "Inspect tasks with `factory tasks --run {}` or `factory inspect <task-id>`.",
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
    let factory = Factory::open(root, fallback_provider())?;
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
    let factory = Factory::open(root, fallback_provider())?;
    let path = factory.create_worktree(task_id)?;
    println!("created worktree at {}", path.display());
    Ok(())
}

fn worktree_remove(root: &Path, task_id: i64) -> Result<()> {
    factory_root(root)?;
    let factory = Factory::open(root, fallback_provider())?;
    factory.remove_worktree(task_id)?;
    println!("removed worktree for task #{task_id}");
    Ok(())
}

fn worktree_status(root: &Path) -> Result<()> {
    factory_root(root)?;
    let factory = Factory::open(root, fallback_provider())?;
    let worktrees = factory
        .list_worktrees()
        .context("not inside a git repository")?;
    for w in worktrees {
        let branch = w.branch.as_deref().unwrap_or("(detached)");
        println!("  {:>3}  {}", w.path.display(), branch);
    }
    Ok(())
}

fn serve(root: &Path, port: u16) -> Result<()> {
    factory_root(root)?;
    let db = FactoryDb::open(&root.join(FACTORY_DIR).join("db.sqlite3"))?;
    let state = factory_api::ApiState {
        db: std::sync::Mutex::new(db),
    };
    let shared = std::sync::Arc::new(state);
    println!("Factory API listening on http://127.0.0.1:{port}");
    println!("Run the dashboard from apps/dashboard with `npm run dev`.");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(factory_api::run_app(shared, port))?;
    Ok(())
}

fn print_run(db: &FactoryDb, run: &factory_models::Run) -> Result<()> {
    let tasks = db.list_tasks(run.id)?;
    let counts = factory_api::types::TaskCounts::from_tasks(&tasks);
    println!(
        "  created {}  model {}  tokens {}",
        run.created_at,
        run.model.as_deref().unwrap_or("_"),
        run.total_tokens
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

fn print_task(task: &factory_models::Task) {
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
