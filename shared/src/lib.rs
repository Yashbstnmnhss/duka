pub mod ast;
pub mod error;
pub mod gc;
pub mod token;
pub mod types;
pub mod utils;
pub mod value;

#[cfg(test)]
mod tests {

    #[test]
    fn visitor_test() {}

    #[test]
    fn gc_test() {
        use crate::gc::*;

        #[derive(Debug)]
        struct A {
            b: B,
        }
        impl Trace for A {
            fn trace(&self, tracer: &mut dyn FnMut(&crate::gc::GcObject)) {
                self.b.trace(tracer);
            }
        }
        #[derive(Debug)]
        struct B {
            c: u32,
        }
        impl Trace for B {
            fn trace(&self, _tracer: &mut dyn FnMut(&crate::gc::GcObject)) {}
        }

        let mut heap = GcHeap::new();

        let a = heap.allocate(A { b: B { c: 0 } });
        let roots = [a.as_gc_object()];
        println!("{:?}", a);

        heap.start_gc(&roots);
    }
}
