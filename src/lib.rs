#![forbid(unsafe_code)]

//! Threadpool provides a way to manage and execute tasks concurrently using a fixed number of worker threads.
//! It allows you to submit tasks that will be executed by one of the available worker threads,
//! providing an efficient way to parallelize work across multiple threads.
//!
//! Maintaining a pool of threads over creating a new thread for each task has the benefit that
//! thread creation and destruction overhead is restricted to the initial creation of the pool.
//!
//! # Examples
//!
//! ```
//! use base_threadpool::{ThreadPool, ThreadPoolBuilder};
//! use std::sync::{Arc, Mutex};
//!
//! let thread_pool = ThreadPoolBuilder::default().build();
//! let value = Arc::new(Mutex::new(0));
//!
//! (0..4).for_each(move |_| {
//!     let value = Arc::clone(&value);
//!     thread_pool.execute(move || {
//!         let mut ir = 0;
//!         (0..100_000_000).for_each(|_| {
//!             ir += 1;
//!         });
//!
//!         let mut lock = value.lock().unwrap();
//!         *lock += ir;
//!     });
//! });
//! ```
use std::{
    num::NonZero,
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
};

/// `ThreadPool` provides a way to manage and execute tasks concurrently using a fixed number of worker threads.
///
/// It allows you to submit tasks that will be executed by one of the available worker threads,
/// providing an efficient way to parallelize work across multiple threads.
#[derive(Debug)]
pub struct ThreadPool {
    workers: Vec<ThreadWorker>,
    producer: Option<mpsc::Sender<ThreadJob>>,
}

impl ThreadPool {
    /// Discovery method for [`ThreadPoolBuilder`].
    ///
    /// Returns a default [`ThreadPoolBuilder`] for constructing a new [`ThreadPool`].
    ///
    /// This method provides a convenient way to start building a `ThreadPool` with default settings,
    /// which can then be customized as needed.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPool;
    ///
    /// let pool = ThreadPool::builder().build();
    /// ```
    pub fn builder() -> ThreadPoolBuilder {
        ThreadPoolBuilder::default()
    }

    /// Schedules a task to be executed by the thread pool.
    ///
    /// Panics if the thread pool has been shut down or if there's an error sending the job.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPool;
    /// use std::sync::{Arc, Mutex};
    ///
    /// // Create a thread pool with 4 worker threads
    /// let pool = ThreadPool::builder().num_threads(4).build();
    ///
    /// // Create a list of items to process
    /// let items = vec!["apple", "banana", "cherry", "date", "elderberry"];
    /// let processed_items = Arc::new(Mutex::new(Vec::new()));
    ///
    /// // Process each item concurrently
    /// for item in items {
    ///     let processed_items = Arc::clone(&processed_items);
    ///     pool.execute(move || {
    ///         // Simulate some processing time
    ///         std::thread::sleep(std::time::Duration::from_millis(100));
    ///
    ///         // Process the item (in this case, convert to uppercase)
    ///         let processed = item.to_uppercase();
    ///
    ///         // Store the processed item
    ///         processed_items.lock().unwrap().push(processed);
    ///     });
    /// }
    /// ```
    #[inline]
    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);

        self.producer
            .as_ref()
            .expect("err acquiring sender ref")
            .send(ThreadJob::Run(job))
            .expect("send error")
    }

    /// Waits for all worker threads in the pool to finish their current tasks and then shuts down the pool.
    ///
    /// This function will block until all workers have completed.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPool;
    /// use std::sync::{
    ///     atomic::{AtomicU16, Ordering},
    ///     Arc,
    /// };
    ///
    /// let mut pool = ThreadPool::builder().build();
    /// let counter = Arc::new(AtomicU16::new(0));
    ///
    /// (0..100).for_each(|_| {
    ///     let counter = Arc::clone(&counter);
    ///     pool.execute(move || {
    ///         let _ = counter.fetch_add(1, Ordering::SeqCst);
    ///     });
    /// });
    ///
    /// pool.join();
    /// assert_eq!(counter.load(Ordering::SeqCst), 100);
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

    /// Provides information about the level of concurrency available in the thread pool.
    ///
    /// Returns the number of worker threads in the pool.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPool;
    ///
    /// let pool = ThreadPool::builder().num_threads(4).build();
    /// assert_eq!(pool.num_threads(), 4);
    /// ```
    #[doc(alias = "available_parallelism")]
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
///
/// # Examples
///
/// Creating a thread pool with default settings:
///
/// ```
/// use base_threadpool::ThreadPoolBuilder;
///
/// let pool = ThreadPoolBuilder::default().build();
/// ```
///
/// Creating a customized thread pool:
///
/// ```
/// use base_threadpool::ThreadPoolBuilder;
///
/// let pool = ThreadPoolBuilder::default()
///     .num_threads(4)
///     .stack_size(3 * 1024 * 1024)
///     .name_prefix("worker".to_string())
///     .build();
/// ```
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
    // # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPoolBuilder;
    ///
    /// let builder = ThreadPoolBuilder::new(4, 2 * 1024 * 1024, "custom-worker".to_string());
    /// let pool = builder.build();
    /// ```
    pub fn new(num_threads: usize, stack_size: usize, name_prefix: String) -> ThreadPoolBuilder {
        assert!(num_threads > 0);

        ThreadPoolBuilder {
            num_threads: NonZero::new(num_threads).unwrap(),
            stack_size: Some(stack_size),
            name_prefix: Some(name_prefix),
        }
    }

    /// Builds and returns a new [`ThreadPool`] instance based on the current configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPoolBuilder;
    ///
    /// let pool = ThreadPoolBuilder::default().num_threads(2).build();
    /// ```
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
    /// # Panics
    ///
    /// Panics if `num_threads` is 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPoolBuilder;
    ///
    /// let builder = ThreadPoolBuilder::default().num_threads(8);
    /// ```
    pub fn num_threads(mut self, num_threads: usize) -> ThreadPoolBuilder {
        assert!(num_threads > 0);

        self.num_threads = NonZero::new(num_threads).unwrap();
        self
    }

    /// Sets the stack size, in bytes, for each thread in the pool.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPoolBuilder;
    ///
    /// let builder = ThreadPoolBuilder::default().stack_size(4 * 1024 * 1024);
    /// ```
    pub fn stack_size(mut self, stack_size: usize) -> ThreadPoolBuilder {
        self.stack_size = Some(stack_size);
        self
    }

    /// Sets the name prefix for threads in the pool.
    ///
    /// # Examples
    ///
    /// ```
    /// use base_threadpool::ThreadPoolBuilder;
    ///
    /// let builder = ThreadPoolBuilder::default().name_prefix("my-worker".to_string());
    /// ```
    pub fn name_prefix(mut self, name_prefix: String) -> ThreadPoolBuilder {
        self.name_prefix = Some(name_prefix);
        self
    }
}

enum ThreadJob {
    Stop,
    Run(Box<dyn FnOnce() + Send + 'static>),
}

#[derive(Debug)]
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

impl std::fmt::Display for ThreadWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]", self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        thread, time,
    };

    mod helpers {
        use super::*;

        const TARGET: usize = 500_000_000;

        pub fn get_sequential_speed() -> time::Duration {
            let mut value = 0;
            let start = time::Instant::now();

            (0..TARGET).for_each(|_| {
                value += 1;
            });

            start.elapsed()
        }

        pub fn get_parallel_speed() -> time::Duration {
            let mut pool = ThreadPoolBuilder::default().build();
            let num_threads = pool.num_threads();
            let value = Arc::new(Mutex::new(0));
            let start = time::Instant::now();

            assert!(num_threads > 0);

            (0..num_threads).for_each(|_| {
                let value = Arc::clone(&value);
                let mut ir = 0;
                pool.execute(move || {
                    (0..TARGET / num_threads).for_each(|_| {
                        ir += 1;
                    });

                    let mut value = value.lock().unwrap();
                    *value += ir;
                });
            });

            pool.join();
            start.elapsed()
        }
    }

    #[test]
    fn construct_pool() {
        let mut pool = ThreadPoolBuilder::default().build();

        let p = Arc::new(Mutex::new(5));
        let v = Arc::clone(&p);
        pool.execute(move || {
            let mut lock = v.lock().unwrap();
            *lock += 1;

            thread::sleep(time::Duration::from_secs(5));
        });

        pool.execute(|| {
            thread::sleep(time::Duration::from_secs(10));
        });

        pool.join();
        assert_eq!(*p.lock().unwrap(), 6);
    }

    #[test]
    fn test_sequential_vs_parallel_speed() {
        let sequential = helpers::get_sequential_speed();
        let parallel = helpers::get_parallel_speed();

        println!("sequential speed: {sequential:#?}\nparallel speed: {parallel:#?}");
        assert!(sequential > parallel);
        assert!(sequential > parallel / 2);
    }

    #[test]
    fn test_join_disposal() {
        let mut pool = ThreadPoolBuilder::default().num_threads(2).build();
        let task_completed = Arc::new(AtomicBool::new(false));
        let task_completed_clone = Arc::clone(&task_completed);

        pool.execute(move || {
            thread::sleep(time::Duration::from_millis(2500));
            task_completed_clone.store(true, Ordering::SeqCst);
        });
        pool.execute(|| {
            thread::sleep(time::Duration::from_secs(1));
        });
        pool.join();

        assert!(
            task_completed.load(Ordering::SeqCst),
            "task not completed before shutdown"
        );
        assert!(
            pool.producer.is_none(),
            "producer isn't none after pool join"
        );
    }

    #[test]
    fn test_setup_builder_default() {
        let pool = ThreadPoolBuilder::default();

        assert_eq!(pool.num_threads, thread::available_parallelism().unwrap());
        assert_eq!(pool.name_prefix, None);
        assert_eq!(pool.stack_size, None);
    }

    #[test]
    fn test_setup_builder_new() {
        let pool = ThreadPoolBuilder::new(1, 5 * 1024, "PrivatePool".to_string());

        assert_eq!(pool.num_threads, NonZero::new(1).unwrap());
        assert_eq!(pool.stack_size, Some(5 * 1024));
        assert_eq!(pool.name_prefix, Some("PrivatePool".to_string()));
    }

    #[test]
    fn test_setup_builder_num_threads() {
        let pool = ThreadPoolBuilder::default().num_threads(4).build();

        assert_eq!(pool.num_threads(), 4);
    }

    #[test]
    fn test_setup_builder_prefix_name() {
        let pool = ThreadPoolBuilder::default().name_prefix("DarkPrivatisedPool".to_string());

        assert_eq!(pool.name_prefix, Some("DarkPrivatisedPool".to_string()));
    }

    #[test]
    fn test_setup_builder_stack_size() {
        let pool = ThreadPoolBuilder::default().stack_size(5 * 1024 * 1024);

        assert_eq!(pool.stack_size, Some(5 * 1024 * 1024));
    }

    #[test]
    #[should_panic(expected = "err acquiring sender ref")]
    fn test_execute_after_join_panics() {
        let mut pool = ThreadPoolBuilder::default().num_threads(2).build();

        pool.join();
        pool.execute(|| {
            println!("shouldn't execute");
        });
    }
}
