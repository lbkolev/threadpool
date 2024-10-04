# Threadpool
Threadpool provides a way to manage and execute tasks concurrently using a fixed number of worker threads. It allows you to submit tasks that will be executed by one of the available worker threads, providing an efficient way to parallelize work across multiple threads.

Maintaining a pool of threads over creating a new thread for each task has the benefit that thread creation and destruction overhead is restricted to the initial creation of the pool.

The implementation is dependency and `unsafe`-free.

## Usage
```rust
use base_threadpool::{ThreadPool, ThreadPoolBuilder};
use std::sync::{Arc, Mutex};

let thread_pool = ThreadPoolBuilder::default().build();
let value = Arc::new(Mutex::new(0));

(0..4).for_each(move |_| {
    let value = Arc::clone(&value);
    thread_pool.execute(move || {
        let mut ir = 0;
        (0..100_000_000).for_each(|_| {
            ir += 1;
        });

        let mut lock = value.lock().unwrap();
        *lock += ir;
    });
});
```

You can create a ThreadPool using the builder pattern:

```rust
use base_threadpool::ThreadPool;

let pool = ThreadPool::builder().build();
let custom_pool = ThreadPool::builder()
    .num_threads(4)
    .stack_size(3 * 1024 * 1024)
    .name_prefix("worker".to_string())
    .build();
```

Use the execute method to submit tasks to the thread pool:
```rust
pool.execute(|| {
    println!("task executed by a thread in the pool");
});
```

To wait for all tasks to complete:
```rust
pool.join();
```

### License
MIT