use std::{
    num::NonZero,
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
};

enum ThreadJob {
    Stop,
    Run(Box<dyn FnOnce() + Send + 'static>),
}

struct ThreadWorker {
    id: usize,
    thread: Option<JoinHandle<()>>,
}
impl ThreadWorker {
    fn new(id: usize, consumer: Arc<Mutex<mpsc::Receiver<ThreadJob>>>) -> ThreadWorker {
        let thread = thread::spawn(move || loop {
            let job = consumer.lock().unwrap().recv().unwrap();

            match job {
                ThreadJob::Run(job) => job(),
                ThreadJob::Stop => {
                    break;
                }
            };
        });

        ThreadWorker {
            id,
            thread: Some(thread),
        }
    }
}

pub struct ThreadPool {
    workers: Vec<ThreadWorker>,
    producer: Option<mpsc::Sender<ThreadJob>>,
}
impl ThreadPool {
    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);

        self.producer
            .as_ref()
            .unwrap()
            .send(ThreadJob::Run(job))
            .unwrap();
    }

    pub fn join(&mut self) {
        (0..self.workers.len()).for_each(|_| {
            self.producer
                .as_ref()
                .unwrap()
                .send(ThreadJob::Stop)
                .unwrap();
        });

        // make sure that the channel gets closed once the thread pool is disposed
        drop(self.producer.take());

        self.workers.iter_mut().for_each(|worker| {
            if let Some(thread) = worker.thread.take() {
                thread.join().unwrap();
            }
        });
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        if self.producer.is_some() {
            // finish up the work that has already been picked up
            self.join();
        }
    }
}

#[derive(Debug)]
pub struct ThreadPoolBuilder {
    num_threads: NonZero<usize>,
}

impl Default for ThreadPoolBuilder {
    fn default() -> ThreadPoolBuilder {
        ThreadPoolBuilder {
            num_threads: thread::available_parallelism().unwrap(),
        }
    }
}

impl ThreadPoolBuilder {
    pub fn build(&self) -> ThreadPool {
        let (producer, consumer) = mpsc::channel();
        let consumer = Arc::new(Mutex::new(consumer));

        let mut workers = Vec::with_capacity(self.num_threads.into());
        (0..self.num_threads.into()).for_each(|id| {
            let consumer = Arc::clone(&consumer);
            let worker = ThreadWorker::new(id, consumer);
            workers.push(worker);
        });

        ThreadPool {
            workers,
            producer: Some(producer),
        }
    }

    pub fn num_threads(&mut self, num_threads: usize) -> &mut ThreadPoolBuilder {
        assert!(num_threads > 0);

        self.num_threads =
            NonZero::new(num_threads).unwrap_or(thread::available_parallelism().unwrap());
        self
    }
}

mod tests {
    use super::*;

    #[test]
    fn construct_pool() {
        let mut pool = ThreadPoolBuilder::default().build();

        let p = Arc::new(Mutex::new(5));
        let v = Arc::clone(&p);
        pool.execute(move || {
            let mut lock = v.lock().unwrap();
            *lock += 1;
        });

        pool.join();
        assert_eq!(*p.lock().unwrap(), 6);
    }
}
