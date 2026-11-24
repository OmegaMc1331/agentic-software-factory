use std::io::{BufRead, Read, Write};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let label = std::env::var("ASF_PROJECT_PROOF").unwrap_or_default();
    if mode == "--version" { println!("controlled-p03-agent 1"); return; }
    if mode == "terminal" {
        println!("terminal-{label}"); std::io::stdout().flush().unwrap();
        for line in std::io::stdin().lock().lines() {
            let line = line.unwrap(); println!("{label}:{line}"); std::io::stdout().flush().unwrap(); if line.trim() == "exit" { return; }
        }
        return;
    }
    let mut mission = String::new(); std::io::stdin().read_to_string(&mut mission).unwrap();
    match mode.as_str() {
        "planner" => println!("{{\"objective\":\"controlled project {label}\",\"tasks\":[{{\"id\":\"T1\",\"title\":\"Project proof\",\"objective\":\"write project-specific evidence\",\"dependencies\":[],\"acceptanceCriteria\":[\"project evidence is recorded\"]}}]}}"),
        "worker" => {
            let cwd = std::env::current_dir().unwrap();
            std::fs::write("project-proof.txt", format!("{label}\n{}\n", cwd.display())).unwrap();
            if let Ok(gate) = std::env::var("ASF_WORKER_GATE") {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
                while !std::path::Path::new(&gate).exists() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(20)); }
                if !std::path::Path::new(&gate).exists() { std::process::exit(2); }
            }
            println!("{{\"commands\":[],\"tests\":[\"controlled fixture {label}\"]}}");
        }
        "reviewer" => println!("{{\"decision\":\"approve\",\"reason\":\"controlled fixture review {label}\"}}"),
        _ => std::process::exit(2),
    }
}
