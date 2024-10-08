use cosmrs::{Any, Gas};
use error_stack::Result;
use thiserror::Error;
use tracing::info;

#[derive(Error, Debug)]
pub enum Error {
    #[error("overflow in gas cost calculation")]
    GasCostOverflow,
}

#[derive(Default)]
pub struct MsgQueue {
    msgs: Vec<Any>,
    gas_cost: Gas,
}

impl MsgQueue {
    pub fn push(&mut self, msg: Any, gas_cost: Gas) -> Result<(), Error> {
        let message_type = msg.type_url.clone();

        self.msgs.push(msg);
        self.gas_cost = self
            .gas_cost
            .checked_add(gas_cost)
            .ok_or(Error::GasCostOverflow)?;

        info!(
            message_type,
            queue_size = self.msgs.len(),
            queue_gas_cost = self.gas_cost,
            "pushed a new message into the queue"
        );

        Ok(())
    }

    pub fn pop_all(&mut self) -> Vec<Any> {
        let msgs = self.msgs.clone();
        self.msgs.clear();
        self.gas_cost = 0;

        msgs
    }

    pub fn gas_cost(&self) -> Gas {
        self.gas_cost
    }

    pub fn len(&self) -> usize {
        self.msgs.len()
    }
}

#[cfg(test)]
mod test {
    use cosmrs::bank::MsgSend;
    use cosmrs::tx::Msg;
    use cosmrs::{AccountId, Any};

    use super::MsgQueue;

    #[test]
    fn msg_queue_push_should_work() {
        let mut queue = MsgQueue::default();
        for gas_cost in 1..5 {
            queue.push(dummy_msg(), gas_cost).unwrap();
        }

        assert_eq!(queue.gas_cost(), 10);
        assert_eq!(queue.msgs.len(), 4);
    }

    #[test]
    fn msg_queue_pop_all_should_work() {
        let mut queue = MsgQueue::default();
        for gas_cost in 1..5 {
            queue.push(dummy_msg(), gas_cost).unwrap();
        }

        assert_eq!(queue.pop_all().len(), 4);
        assert_eq!(queue.gas_cost(), 0);
        assert_eq!(queue.msgs.len(), 0);
    }

    fn dummy_msg() -> Any {
        MsgSend {
            from_address: AccountId::new("", &[1, 2, 3]).unwrap(),
            to_address: AccountId::new("", &[4, 5, 6]).unwrap(),
            amount: vec![],
        }
        .to_any()
        .unwrap()
    }
}
