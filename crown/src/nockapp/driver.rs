use crate::noun::slab::NounSlab;
use futures::future::Future;
use std::pin::Pin;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinSet;

use super::error::NockAppError;

pub type IODriverFuture = Pin<Box<dyn Future<Output = Result<(), NockAppError>> + Send>>;
pub type IODriverFn = Box<dyn FnOnce(NockAppHandle) -> IODriverFuture>;
pub type TaskJoinSet = JoinSet<Result<(), NockAppError>>;
pub type ActionSender = mpsc::Sender<IOAction>;
pub type ActionReceiver = mpsc::Receiver<IOAction>;
pub type EffectSender = broadcast::Sender<NounSlab>;
pub type EffectReceiver = broadcast::Receiver<NounSlab>;

/// Result of a poke: either Ack if it succeeded or Nack if it failed
#[derive(Debug)]
pub enum PokeResult {
    Ack,
    Nack,
}

pub enum Operation {
    Poke,
    Peek,
}

pub fn make_driver<F, Fut>(f: F) -> IODriverFn
where
    F: FnOnce(NockAppHandle) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), NockAppError>> + Send + 'static,
{
    Box::new(move |handle| Box::pin(f(handle)))
}

pub struct NockAppHandle {
    pub io_sender: ActionSender,
    pub effect_sender: EffectSender,
    pub effect_receiver: Mutex<EffectReceiver>,
    pub exit: mpsc::Sender<usize>,
}

/// IO actions sent between [`NockAppHandle`] and [`crate::NockApp`] over channels.
///
/// Used by [`NockAppHandle`] to send poke/peek requests to [`crate::NockApp`] ,
/// which processes them against the Nock kernel and returns results
/// via oneshot channels.
#[derive(Debug)]
pub enum IOAction {
    /// Poke request to [`crate::NockApp`]
    Poke {
        wire: NounSlab,
        poke: NounSlab,
        ack_channel: oneshot::Sender<PokeResult>,
    },
    /// Peek request to [`crate::NockApp`]
    Peek {
        path: NounSlab,
        result_channel: oneshot::Sender<Option<NounSlab>>,
    },
}

impl NockAppHandle {
    pub async fn poke(&self, wire: NounSlab, poke: NounSlab) -> Result<PokeResult, NockAppError> {
        let (ack_channel, ack_future) = oneshot::channel();
        self.io_sender
            .send(IOAction::Poke {
                wire,
                poke,
                ack_channel,
            })
            .await?;
        Ok(ack_future.await?)
    }

    pub async fn peek(&self, path: NounSlab) -> Result<Option<NounSlab>, NockAppError> {
        let (result_channel, result_future) = oneshot::channel();
        self.io_sender
            .send(IOAction::Peek {
                path,
                result_channel,
            })
            .await?;
        Ok(result_future.await?)
    }

    pub async fn next_effect(&self) -> Result<NounSlab, NockAppError> {
        let mut effect_receiver = self.effect_receiver.lock().await;
        Ok(effect_receiver.recv().await?)
    }

    pub fn dup(self) -> (Self, Self) {
        let io_sender = self.io_sender.clone();
        let effect_sender = self.effect_sender.clone();
        let effect_receiver = Mutex::new(effect_sender.subscribe());
        let exit = self.exit.clone();
        (
            self,
            NockAppHandle {
                io_sender,
                effect_sender,
                effect_receiver,
                exit,
            },
        )
    }

    pub fn clone_io_sender(&self) -> ActionSender {
        self.io_sender.clone()
    }
}
