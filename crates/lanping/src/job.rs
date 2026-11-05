//! The bookkeeping of one background job: which run is the current one, and how to stop it.
//!
//! A tab that starts work in the background gives each run a number. An answer that comes
//! back carries the number of the run that asked, and is ignored when a newer run, or a stop,
//! has happened since. Cancelling also stops the work, so a replaced run does not keep going.

use std::future::Future;

use iced::Task;
use iced::task::Handle;

use crate::app::Message;

#[derive(Debug, Default)]
pub struct Job {
    generation: u64,
    handle: Option<Handle>,
}

impl Job {
    /// Whether an answer belongs to the run that is current.
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// Stops the current run, if any. Answers it has not delivered yet are ignored.
    pub fn cancel(&mut self) {
        self.generation += 1;
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }

    /// Starts `work` as the new current run, stopping the one before. The answer comes back
    /// as the message `tag` builds from the run number and the answer.
    pub fn spawn<T, F>(
        &mut self,
        work: F,
        tag: impl Fn(u64, T) -> Message + Send + 'static,
    ) -> Task<Message>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        self.cancel();
        self.spawn_next(work, tag)
    }

    /// Starts `work` as the next step of the run that is current, for a run that goes on in
    /// steps, such as a measurement that repeats. The run keeps its number.
    pub fn spawn_next<T, F>(
        &mut self,
        work: F,
        tag: impl Fn(u64, T) -> Message + Send + 'static,
    ) -> Task<Message>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let generation = self.generation;
        let (task, handle) = Task::perform(work, move |answer| tag(generation, answer)).abortable();
        self.handle = Some(handle);
        task
    }

    /// Records that the current run has delivered its answer, so there is nothing to stop.
    pub fn finished(&mut self) {
        self.handle = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_run_is_the_only_current_one() {
        let mut job = Job::default();
        let _ = job.spawn(async {}, |_, ()| Message::Noop);
        assert!(job.is_current(1));
        let _ = job.spawn(async {}, |_, ()| Message::Noop);
        assert!(!job.is_current(1));
        assert!(job.is_current(2));
    }

    #[test]
    fn cancelling_makes_the_run_stale() {
        let mut job = Job::default();
        let _ = job.spawn(async {}, |_, ()| Message::Noop);
        job.cancel();
        assert!(!job.is_current(1));
    }

    #[test]
    fn nothing_is_current_before_the_first_run() {
        let job = Job::default();
        assert!(!job.is_current(1));
        assert!(
            job.is_current(0),
            "run 0 is the empty state, never handed out"
        );
    }
}
