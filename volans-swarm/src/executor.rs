use std::pin::Pin;

pub trait Executor {
    #[track_caller]
    fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>);
}
