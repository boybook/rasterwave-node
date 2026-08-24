use std::sync::OnceLock;

use napi::{Error, Result, Status};
use rayon::{ThreadPool, ThreadPoolBuilder};

pub const MAX_OPERATIONS: usize = 64;
pub const MAX_INPUT_OPERATIONS: usize = MAX_OPERATIONS - 4;
pub const MAX_READ_SAMPLES: u32 = 1_048_576;

pub fn pool() -> &'static ThreadPool {
    static POOL: OnceLock<ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, 4);
        ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("rasterwave-node-{index}"))
            .build()
            .expect("rasterwave native thread pool must initialize")
    })
}

pub fn error(code: &str, message: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, format!("{code}: {message}"))
}

pub fn lock_error() -> Error {
    error("RASTERWAVE_INTERNAL", "native session lock was poisoned")
}

pub fn safe_number(value: u64, field: &str) -> Result<f64> {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    if value > MAX_SAFE_INTEGER {
        return Err(error(
            "RASTERWAVE_INTEGER_OVERFLOW",
            format!("{field} exceeds Number.MAX_SAFE_INTEGER"),
        ));
    }
    Ok(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_is_bounded_and_reused() {
        assert!(pool().current_num_threads() <= 4);
        assert!(std::ptr::eq(pool(), pool()));
    }

    #[test]
    fn safe_number_rejects_precision_loss() {
        assert_eq!(
            safe_number(9_007_199_254_740_991, "value").unwrap(),
            9_007_199_254_740_991.0
        );
        assert!(safe_number(9_007_199_254_740_992, "value").is_err());
    }
}
