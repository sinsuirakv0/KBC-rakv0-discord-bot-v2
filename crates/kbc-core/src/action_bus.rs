use kbc_protocol::CoreAction;
use tokio::sync::{Mutex, mpsc};

pub(crate) struct ActionBus {
    receiver: Mutex<mpsc::Receiver<CoreAction>>,
}

impl ActionBus {
    pub(crate) fn new(capacity: usize) -> (Self, mpsc::Sender<CoreAction>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                receiver: Mutex::new(receiver),
            },
            sender,
        )
    }

    pub(crate) async fn next_action(&self) -> Option<CoreAction> {
        self.receiver.lock().await.recv().await
    }
}
