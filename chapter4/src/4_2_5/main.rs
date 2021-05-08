#![allow(dead_code)]

use std::any::Any;
use std::fmt::Debug;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq)]
enum ContextError {
    Canceled,
}

#[derive(Debug, Clone)]
enum ContextValueError {
    NotFound,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Hash)]
enum ContextKey {
    String(String),
    CancelContext,
}

trait Context: Send + Sync {
    fn deadline(&self, deadline: Instant, ok: bool);
    fn done(&self) -> Pin<Box<dyn Future<Output = Result<(), ContextError>> + '_>>;
    fn err(&self) -> Option<ContextError>;
    fn value(&self, key: &ContextKey) -> Result<Arc<dyn Any>, ContextValueError>;
}

trait Canceler: Send + Sync {
    fn cancel(&self, remove_from_parent: bool, error: ContextError);
    fn done(&self) -> Pin<Box<dyn Future<Output = Result<(), ContextError>> + '_>>;
}

trait HasContextBody {
    fn context_body(&self) -> Arc<Mutex<ContextBody>>;
}

struct ContextBody {
    children: Vec<Arc<dyn Canceler>>,
    parent: Option<Arc<dyn Context>>,
    canceled: Option<ContextError>,
}

struct WithCancel {
    cancel_notify: Notify,
    body: Arc<Mutex<ContextBody>>,
}

struct CancelFunc<C: Canceler> {
    context: Arc<C>,
}

impl<C: Canceler> CancelFunc<C> {
    pub fn cancel(&self) {
        self.context.cancel(true, ContextError::Canceled)
    }
}

impl WithCancel {
    pub fn new<C: 'static + HasContextBody + Context>(
        context: Arc<C>,
    ) -> (Arc<Self>, CancelFunc<Self>) {
        let this = Arc::new(Self {
            cancel_notify: Notify::new(),
            body: Arc::new(Mutex::new(ContextBody {
                canceled: None,
                children: vec![],
                parent: Some(context.clone()),
            })),
        });
        context
            .context_body()
            .lock()
            .unwrap()
            .children
            .push(this.clone());
        (this.clone(), CancelFunc { context: this })
    }

    pub async fn done(&self) -> Result<(), ContextError> {
        let _ = self.cancel_notify.notified().await;
        if let Some(e) = self.body.lock().unwrap().canceled.clone() {
            Err(e)
        } else {
            Ok(())
        }
    }
}

impl Context for WithCancel {
    fn deadline(&self, _deadline: Instant, _ok: bool) {
        todo!()
    }

    fn done(&self) -> Pin<Box<dyn Future<Output = Result<(), ContextError>> + '_>> {
        Box::pin(WithCancel::done(self))
    }

    fn err(&self) -> Option<ContextError> {
        self.body.lock().unwrap().canceled.clone()
    }

    fn value(&self, key: &ContextKey) -> Result<Arc<dyn Any>, ContextValueError> {
        self.body
            .lock()
            .unwrap()
            .parent
            .as_ref()
            .expect("WithCancelは必ず親を持つ")
            .value(key)
    }
}

impl Canceler for WithCancel {
    fn cancel(&self, _remove_from_parent: bool, error: ContextError) {
        let mut body = self.body.lock().unwrap();

        for child in &body.children {
            child.cancel(false, error.clone())
        }
        body.canceled.replace(error);

        self.cancel_notify.notify_waiters();
    }

    fn done(&self) -> Pin<Box<dyn Future<Output = Result<(), ContextError>> + '_>> {
        Context::done(self)
    }
}

struct Background {
    body: Arc<Mutex<ContextBody>>,
}

impl Background {
    fn new() -> Arc<Self> {
        Arc::new(Background {
            body: Arc::new(Mutex::new(ContextBody {
                parent: None,
                children: vec![],
                canceled: None,
            })),
        })
    }
}

impl Context for Background {
    fn deadline(&self, _deadline: Instant, _ok: bool) {
        todo!()
    }

    fn done(&self) -> Pin<Box<dyn Future<Output = Result<(), ContextError>>>> {
        todo!()
    }

    fn err(&self) -> Option<ContextError> {
        todo!()
    }

    fn value(&self, _key: &ContextKey) -> Result<Arc<dyn Any>, ContextValueError> {
        Err(ContextValueError::NotFound)
    }
}

impl HasContextBody for Background {
    fn context_body(&self) -> Arc<Mutex<ContextBody>> {
        self.body.clone()
    }
}

#[tokio::main]
async fn main() {
    println!("start sub()");

    let (ctx, canceler) = WithCancel::new(Background::new()); // Backgroundはrootとなる空のContext

    tokio::spawn(async move {
        println!("sub() is finished");
        canceler.cancel();
    });

    let done = ctx.done().await;
    assert_eq!(done.unwrap_err(), ContextError::Canceled);
    println!("all tasks are finished");
}
