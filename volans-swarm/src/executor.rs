use futures::FutureExt;
use std::pin::Pin;

pub trait Executor {
    #[track_caller]
    fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>);
}

pub struct ExecSwitch(Box<dyn Executor + Send>);

impl ExecSwitch {
    pub fn boxed<T>(executor: T) -> Self
    where
        T: Executor + Send + 'static,
    {
        ExecSwitch(Box::new(executor))
    }

    pub fn new(executor: Box<dyn Executor + Send>) -> Self {
        ExecSwitch(executor)
    }

    pub fn spawn(&mut self, task: impl Future<Output = ()> + Send + 'static) {
        self.0.exec(task.boxed());
    }
}
