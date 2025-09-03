pub mod core;

pub use core::{
    GcContext, GcObj, GcPtr, add_root, add_temp_root, clear_temp_root, collect_garbage,
    global_alloc, remove_root,
};

#[cfg(test)]
mod tests {
    #[test]
    fn gc_test() {}
}
