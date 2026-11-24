use std::io::Read;
fn main(){
    let mut prompt=String::new();std::io::stdin().read_to_string(&mut prompt).unwrap();
    let mode=std::env::args().nth(1).unwrap_or_default();
    if prompt.is_empty() {
        prompt = std::env::args().find(|arg| arg.starts_with("ASF_ORCHESTRATOR_V1")).unwrap_or_default();
    }
    if let Ok(path)=std::env::var("P07_WAIT") {if mode=="worker" {while !std::path::Path::new(&path).exists(){std::thread::sleep(std::time::Duration::from_millis(20));}}}
    if mode=="planner"&&prompt.starts_with("ASF_ORCHESTRATOR_V1") {
        if std::env::var("P07_CLASSIFY_FAIL").is_ok(){eprintln!("controlled classifier process failure");std::process::exit(17);}
        if std::env::var("P07_CODEX_QUOTA").is_ok(){
            eprintln!("OpenAI Codex v0.0.0\nERROR: You've hit your usage limit.");
            std::process::exit(1);
        }
        if prompt.contains("WAIT_FOR_STOP") {
            if let Ok(marker) = std::env::var("P07_CHAT_STARTED") {
                std::fs::write(marker, std::process::id().to_string()).unwrap();
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
        }
        let data=prompt.split("UNTRUSTED_DATA_JSON\n").nth(1).unwrap_or("");
        let user=data.rsplit_once(",\"role\":\"user\"").map(|(before,_)|before.rsplit_once("\"content\":").map(|(_,text)|text).unwrap_or("")).unwrap_or("");
        if data.contains("INVALID_ACTION"){println!("{{\"action\":\"shell\",\"command\":\"forbidden\"}}");}
        else if user.contains("INSPECT_RUN_1"){println!("{{\"action\":\"inspect_mission\",\"run_id\":1}}");}
        else if user.contains("START_MISSION"){println!("{{\"action\":\"create_mission\",\"objective\":\"Write controlled project proof\"}}");}
        else {println!("{{\"action\":\"respond\",\"content\":\"Controlled agent response; protocol fixture, not a real model.\"}}");}
        return;
    }
    if mode=="planner" {
        if std::env::var("P07_INVALID").is_ok(){println!("{{\"objective\":\"invalid\",\"tasks\":[{{\"id\":\"a\",\"title\":\"bad\",\"objective\":\"bad\",\"dependencies\":[\"a\"],\"acceptanceCriteria\":[\"bad\"]}}]}}");return;}
        println!("{{\"objective\":\"Controlled proof\",\"tasks\":[{{\"id\":\"T1\",\"title\":\"Proof\",\"objective\":\"write proof\",\"dependencies\":[],\"acceptanceCriteria\":[\"proof exists\"]}}]}}");
    } else if mode=="worker" {
        if std::env::var("P07_FAIL").is_ok(){println!("controlled failing test");std::process::exit(2);}
        std::fs::write("proof.txt",format!("{}\n",std::env::current_dir().unwrap().display())).unwrap();
        println!("{{\"commands\":[],\"tests\":[]}}");
    } else if mode=="reviewer" {println!("{{\"decision\":\"approve\",\"reason\":\"Controlled deterministic review\"}}");}
    else {std::process::exit(2);}
}
