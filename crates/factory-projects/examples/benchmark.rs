use factory_projects::{ProjectLimits, ProjectManager};
use serde_json::json;
use std::path::Path;
use std::time::Instant;

fn milliseconds(work: impl FnOnce()) -> f64 {
    let start = Instant::now();
    work();
    start.elapsed().as_secs_f64() * 1000.0
}
fn manager(root: &Path) -> ProjectManager {
    ProjectManager::new(
        &root.join("application/registry.sqlite3"),
        &[root.to_owned()],
        ProjectLimits::default(),
    )
    .unwrap()
}
fn memory() -> serde_json::Value {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = unsafe {
            GetProcessMemoryInfo(GetCurrentProcess(), &mut counters as *mut _ as *mut _, size)
        };
        if ok != 0 {
            return json!({"private_bytes":counters.PrivateUsage,"working_set_bytes":counters.WorkingSetSize});
        }
    }
    json!({"unavailable":true})
}
fn main() {
    let temporary = tempfile::TempDir::new().unwrap();
    let root = temporary.path();
    let mut ids = Vec::new();
    let mut measured = Vec::new();
    let mut projects = manager(root);
    for i in 0..8 {
        let path = root.join(format!("project-{i}"));
        std::fs::create_dir(&path).unwrap();
        factory_git::Repo::initialize(&path).unwrap();
        factory_core::Factory::init(&path).unwrap();
        let entry = projects.register(&path, None).unwrap();
        ids.push(entry.project_id);
        if i == 0 || i == 7 {
            drop(projects);
            let mut times = Vec::new();
            for _ in 0..9 {
                times.push(milliseconds(|| {
                    let fresh = manager(root);
                    drop(fresh);
                }));
            }
            projects = manager(root);
            let lists: Vec<_> = (0..9)
                .map(|_| {
                    milliseconds(|| {
                        assert_eq!(projects.list().unwrap().len(), i + 1);
                    })
                })
                .collect();
            measured.push(json!({"registered":i+1,"manager_start_ms":times,"availability_list_ms":lists,"open_runtimes":0}));
        }
    }
    let closed = memory();
    let mut open = Vec::new();
    let open_times: Vec<_> = ids
        .iter()
        .map(|id| milliseconds(|| open.push(projects.open(id).unwrap())))
        .collect();
    let loaded = memory();
    let reuse: Vec<_> = (0..30)
        .map(|i| {
            milliseconds(|| {
                projects.context(&ids[i % ids.len()]).unwrap();
            })
        })
        .collect();
    let duplicate: Vec<_> = (0..9)
        .map(|_| {
            milliseconds(|| {
                projects.register(&open[0].root, None).unwrap();
            })
        })
        .collect();
    let root_a = open[0].root.clone();
    drop(open);
    for id in &ids {
        projects.close(id).unwrap();
    }
    let external = factory_runtime::Runtime::new(&root_a).unwrap();
    let contention = milliseconds(|| {
        assert!(projects.open(&ids[0]).is_err());
    });
    drop(external);
    let mut reopened = Vec::new();
    for _ in 0..9 {
        reopened.push(milliseconds(|| {
            projects.open(&ids[0]).unwrap();
        }));
        projects.close(&ids[0]).unwrap();
    }
    println!("{}",serde_json::to_string_pretty(&json!({"schema":1,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"profile":"unoptimized, debug info disabled","fixtures":{"projects":8,"files_per_project":"Git metadata and initialized Factory only","synthetic":true},"startup":measured,"open_project_ms":open_times,"context_lookup_ms":reuse,"duplicate_registration_ms":duplicate,"registry_write_ms":(0..9).map(|i|milliseconds(||{projects.rename(&ids[0],&format!("Name {i}")).unwrap();})).collect::<Vec<_>>(),"warm_reopen_ms":reopened,"lock_refusal_ms":contention,"memory_before_open":closed,"memory_eight_open":loaded,"limits":"process aggregate; allocator and database caches are included; not isolated per-runtime RSS"})).unwrap());
}
