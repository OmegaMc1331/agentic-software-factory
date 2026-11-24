use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

pub(crate) struct WorkerBarrier {
    pub(crate) port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl WorkerBarrier {
    pub(crate) fn new(expected: usize) -> Self {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(120);
            let mut pending = Vec::new();
            let mut arrivals = 0;
            while !cancelled.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        arrivals += 1;
                        if arrivals >= 2 {
                            stream.write_all(b"G").unwrap();
                            for mut waiting in pending.drain(..) {
                                std::io::Write::write_all(&mut waiting, b"G").unwrap();
                            }
                        } else {
                            pending.push(stream);
                        }
                        if arrivals == expected {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("Controlled stress barrier: {error}"),
                }
            }
        });
        Self {
            port,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for WorkerBarrier {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.thread.take().unwrap().join();
    }
}
