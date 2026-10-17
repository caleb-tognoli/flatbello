//! Where the engine runs: in a worker process (the default), or in this process (tests and
//! headless rendering). Either way a background thread owns it and the UI sends it jobs.
//!
//! The worker is this executable started with `--worker`, speaking the framed protocol of
//! [`flatbello_engine::wire`] over stdin and stdout. If it dies (OCCT can crash on a broken file),
//! the job fails with [`Crashed`] and the next job starts a fresh worker.

use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use flatbello_core::LengthUnit;
use flatbello_engine::wire::{
    Request, Response, Service, decode_response, encode_request, read_frame, write_frame,
};

/// The engine stopped while handling a job; its open model is gone.
#[derive(Clone, Debug)]
pub struct Crashed(pub String);

pub type Reply = Result<Response, Crashed>;

pub enum Job {
    /// Reads the file here (the engine only sees bytes) and opens it.
    Open {
        path: PathBuf,
        units: Option<LengthUnit>,
    },
    Call(Request),
}

pub struct Backend {
    jobs: Sender<(Job, Sender<Reply>)>,
}

impl Backend {
    /// Runs the engine on a thread of this process.
    pub fn in_process() -> Backend {
        let mut service = Service::default();
        Backend::spawn(move |request| Ok(service.handle(request)))
    }

    /// Runs the engine in child processes of `exe --worker`, restarting them as needed.
    pub fn worker(exe: PathBuf) -> Backend {
        let mut worker: Option<Worker> = None;
        Backend::spawn(move |request| {
            let w = match &mut worker {
                Some(w) => w,
                None => worker.insert(Worker::start(&exe).map_err(Crashed)?),
            };
            match w.call(&request) {
                Ok(r) => Ok(r),
                Err(e) => {
                    if let Some(mut dead) = worker.take() {
                        let _ = dead.child.kill();
                        let _ = dead.child.wait();
                    }
                    Err(Crashed(e))
                }
            }
        })
    }

    fn spawn(mut handle: impl FnMut(Request) -> Reply + Send + 'static) -> Backend {
        let (tx, rx) = mpsc::channel::<(Job, Sender<Reply>)>();
        let spawned = std::thread::Builder::new()
            .name("flatbello-backend".into())
            .spawn(move || {
                for (job, reply) in rx {
                    let request = match job {
                        Job::Call(r) => r,
                        Job::Open { path, units } => match std::fs::read(&path) {
                            Ok(bytes) => Request::Open {
                                bytes,
                                hint: path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                                units,
                            },
                            Err(e) => {
                                let msg = format!("reading {}: {e}", path.display());
                                let _ = reply.send(Ok(Response::Error(msg)));
                                continue;
                            }
                        },
                    };
                    let _ = reply.send(handle(request));
                }
            });
        if let Err(e) = spawned {
            eprintln!("flatbello: cannot start the engine thread: {e}");
        }
        Backend { jobs: tx }
    }

    /// Queues a job; the reply arrives on the returned channel.
    pub fn send(&self, job: Job) -> Receiver<Reply> {
        let (tx, rx) = mpsc::channel();
        if self.jobs.send((job, tx.clone())).is_err() {
            let _ = tx.send(Err(Crashed("the engine thread is gone".into())));
        }
        rx
    }

    /// Runs a request and waits for its reply.
    pub fn call(&self, request: Request) -> Reply {
        self.send(Job::Call(request))
            .recv()
            .unwrap_or_else(|_| Err(Crashed("the engine thread is gone".into())))
    }
}

struct Worker {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl Worker {
    fn start(exe: &PathBuf) -> Result<Worker, String> {
        let mut command = Command::new(exe);
        command
            .arg("--worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console window for the worker.
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot start the engine process: {e}"))?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err("the engine process has no pipes".into());
        };
        Ok(Worker {
            child,
            input: BufWriter::new(input),
            output: BufReader::new(output),
        })
    }

    fn call(&mut self, request: &Request) -> Result<Response, String> {
        let lost = |what: &str| format!("the engine process stopped ({what})");
        write_frame(&mut self.input, &encode_request(request)).map_err(|e| lost(&e.to_string()))?;
        let frame = read_frame(&mut self.output)
            .map_err(|e| lost(&e.to_string()))?
            .ok_or_else(|| lost("no reply"))?;
        decode_response(&frame).map_err(|e| lost(&e.to_string()))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app binary next to the test executable (target/debug/flatbello), built by cargo for
    /// the integration tests.
    fn app_exe() -> Option<PathBuf> {
        let deps = std::env::current_exe().ok()?;
        let dir = deps.parent()?.parent()?;
        let exe = dir.join(format!("flatbello{}", std::env::consts::EXE_SUFFIX));
        exe.is_file().then_some(exe)
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/generated")
            .join(name)
    }

    #[test]
    fn worker_backend_recovers_from_a_crash() {
        let Some(exe) = app_exe() else {
            eprintln!("app binary not built; skipping");
            return;
        };
        let backend = Backend::worker(exe);
        let open = || Job::Open {
            path: fixture("plate_holes.step"),
            units: None,
        };
        let first = backend.send(open()).recv().unwrap();
        assert!(matches!(first, Ok(Response::Opened(_))));
        assert!(matches!(backend.call(Request::Crash), Err(Crashed(_))));
        // The next job starts a fresh worker.
        let again = backend.send(open()).recv().unwrap();
        assert!(matches!(again, Ok(Response::Opened(_))));
    }

    #[test]
    fn in_process_backend_reports_missing_files() {
        let backend = Backend::in_process();
        let reply = backend
            .send(Job::Open {
                path: PathBuf::from("/no/such/file.step"),
                units: None,
            })
            .recv()
            .unwrap();
        assert!(matches!(reply, Ok(Response::Error(e)) if e.contains("reading")));
    }
}
