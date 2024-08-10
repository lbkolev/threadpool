//! Threadpools.

use std::{
    num::NonZero,
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
};

/// Minimalistic
pub struct ThreadPool {
    workers: Vec<ThreadWorker>,
    producer: Option<mpsc::Sender<ThreadJob>>,
}
impl ThreadPool {
    /// Discovery method for [`ThreadPoolBuilder`].
    ///
    /// Provides the default [`ThreadPoolBuilder`], ready for consumption.
    pub fn builder() -> ThreadPoolBuilder {
        ThreadPoolBuilder::default()
    }

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

    /// Waits for the pool-associated threads to finish.
    ///
    /// This function will return immediately after all the threads have already finished.
    ///
    /// # Examples
    ///
    /// ```
    /// use thr_pool::{ThreadPool, ThreadPoolBuilder};
    ///
    /// let mut thread_pool = ThreadPoolBuilder::default().build();
    ///
    /// thread_pool.execute(|| {
    ///     println!("In threadpool");
    /// });
    ///
    /// thread_pool.join();
    /// println!("threadpool finished executing");
    /// ```
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

    #[doc(alias = "available_concurrency")]
    #[doc(alias = "available_workers")]
    #[doc(alias = "available_threads")]
    pub fn num_threads(&self) -> usize {
        self.workers.len()
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

/// A builder for configuring and creating a [`ThreadPool`].
///
/// This builder allows you to set various parameters for the thread pool,
/// such as the number of threads, stack size, and a name prefix for the threads.
#[derive(Debug)]
pub struct ThreadPoolBuilder {
    num_threads: NonZero<usize>,
    stack_size: Option<usize>,
    name_prefix: Option<String>,
}

/// Default parameters for [`ThreadPoolBuilder`]
/// The Default number of threads available for the [`ThreadPool`] is [`std::thread::available_parallelism`].
impl Default for ThreadPoolBuilder {
    fn default() -> ThreadPoolBuilder {
        ThreadPoolBuilder {
            num_threads: thread::available_parallelism().unwrap(),
            stack_size: Option::default(),
            name_prefix: Option::default(),
        }
    }
}

impl ThreadPoolBuilder {
    /// Constructs a new instance of [`ThreadPoolBuilder`] with specified parameters.
    ///
    /// # Arguments
    ///
    /// * `num_threads` - The number of threads in the pool. Must be greater than 0.
    /// * `stack_size` - The stack size for each thread in bytes.
    /// * `name_prefix` - A prefix for naming the threads in the pool.
    ///
    /// # Panics
    ///
    /// Panics if `num_threads` is 0.
    pub fn new(num_threads: usize, stack_size: usize, name_prefix: String) -> ThreadPoolBuilder {
        assert!(num_threads > 0);

        ThreadPoolBuilder {
            num_threads: NonZero::new(num_threads).unwrap(),
            stack_size: Some(stack_size),
            name_prefix: Some(name_prefix),
        }
    }

    /// Builds and returns a new [`ThreadPool`] instance based on the current configuration.
    pub fn build(&self) -> ThreadPool {
        let (producer, consumer) = mpsc::channel();
        let consumer = Arc::new(Mutex::new(consumer));

        let mut workers = Vec::with_capacity(self.num_threads.into());
        (0..self.num_threads.into()).for_each(|id| {
            let consumer = Arc::clone(&consumer);
            let mut builder = thread::Builder::new();

            if let Some(stack_size) = self.stack_size {
                builder = builder.stack_size(stack_size);
            }

            if let Some(prefix) = &self.name_prefix {
                builder = builder.name(format!("{}-{}", prefix, id));
            }

            let worker = ThreadWorker::new(id, consumer, builder);
            workers.push(worker);
        });

        ThreadPool {
            workers,
            producer: Some(producer),
        }
    }

    /// Sets the number of threads for the thread pool.
    ///
    /// # Arguments
    ///
    /// * `num_threads` - The number of threads to use. Must be greater than 0.
    ///
    /// # Panics
    ///
    /// Panics if `num_threads` is 0.
    pub fn num_threads(mut self, num_threads: usize) -> ThreadPoolBuilder {
        assert!(num_threads > 0);

        self.num_threads = NonZero::new(num_threads).unwrap();
        self
    }

    /// Sets the stack size for each thread in the pool.
    ///
    /// # Arguments
    ///
    /// * `stack_size` - The stack size in bytes for each thread.
    pub fn stack_size(mut self, stack_size: usize) -> ThreadPoolBuilder {
        self.stack_size = Some(stack_size);
        self
    }

    /// Sets the name prefix for threads in the pool.
    ///
    /// Each thread will be named with this prefix followed by a unique number.
    ///
    /// # Arguments
    ///
    /// * `name_prefix` - The prefix to use for thread names.¡
    pub fn name_prefix(mut self, name_prefix: String) -> ThreadPoolBuilder {
        self.name_prefix = Some(name_prefix);
        self
    }
}

enum ThreadJob {
    Stop,
    Run(Box<dyn FnOnce() + Send + 'static>),
}

struct ThreadWorker {
    id: usize,
    thread: Option<JoinHandle<()>>,
}
impl ThreadWorker {
    fn new(
        id: usize,
        consumer: Arc<Mutex<mpsc::Receiver<ThreadJob>>>,
        builder: thread::Builder,
    ) -> ThreadWorker {
        let thread = builder
            .spawn(move || loop {
                let job = consumer.lock().unwrap().recv().unwrap();

                match job {
                    ThreadJob::Run(job) => job(),
                    ThreadJob::Stop => break,
                };
            })
            .unwrap();

        ThreadWorker {
            id,
            thread: Some(thread),
        }
    }
}

#[cfg(test)]
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

            thread::sleep(std::time::Duration::from_secs(5));
        });

        pool.execute(|| {
            thread::sleep(std::time::Duration::from_secs(10));
        });

        pool.join();
        assert_eq!(*p.lock().unwrap(), 6);
    }
}
