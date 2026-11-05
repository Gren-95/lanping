//! The Web tab state: a website's answer and a domain's email records.

use iced::Task;
use lanping_core::dns::DnsError;
use lanping_core::email_check::{self, EmailReport};
use lanping_core::website_check::{self, WebsiteError, WebsiteReport};

use crate::app::Message;
use crate::job::Job;

/// The pages of the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebPage {
    #[default]
    Website,
    Email,
}

impl WebPage {
    pub const ALL: [Self; 2] = [Self::Website, Self::Email];

    pub fn title(self) -> &'static str {
        match self {
            Self::Website => "Website",
            Self::Email => "Email records",
        }
    }
}

pub enum Lookup<T, E> {
    Idle,
    Running,
    Done(T),
    Failed(E),
}

impl<T, E> Lookup<T, E> {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    fn settle(result: Result<T, E>) -> Self {
        match result {
            Ok(value) => Self::Done(value),
            Err(error) => Self::Failed(error),
        }
    }
}

/// What is typed, and what the last checks found.
pub struct WebState {
    pub page: WebPage,
    pub address: String,
    pub domain: String,
    pub site: Lookup<WebsiteReport, WebsiteError>,
    pub email: Lookup<EmailReport, DnsError>,
    site_job: Job,
    email_job: Job,
}

impl WebState {
    pub fn new() -> Self {
        Self {
            page: WebPage::default(),
            address: String::new(),
            domain: String::new(),
            site: Lookup::Idle,
            email: Lookup::Idle,
            site_job: Job::default(),
            email_job: Job::default(),
        }
    }

    pub fn check_site(&mut self) -> Task<Message> {
        self.site = Lookup::Running;
        let address = self.address.clone();
        self.site_job.spawn(
            async move { website_check::check(&address).await },
            Message::WebDone,
        )
    }

    pub fn finish_site(&mut self, generation: u64, result: Result<WebsiteReport, WebsiteError>) {
        if self.site_job.is_current(generation) {
            self.site_job.finished();
            self.site = Lookup::settle(result);
        }
    }

    pub fn check_email(&mut self) -> Task<Message> {
        self.email = Lookup::Running;
        let domain = self.domain.clone();
        self.email_job.spawn(
            async move { email_check::check(&domain).await },
            Message::EmailDone,
        )
    }

    pub fn finish_email(&mut self, generation: u64, result: Result<EmailReport, DnsError>) {
        if self.email_job.is_current(generation) {
            self.email_job.finished();
            self.email = Lookup::settle(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overtaken_site_answer_is_ignored() {
        let mut state = WebState::new();
        let _ = state.check_site();
        let _ = state.check_site();
        state.finish_site(1, Err(WebsiteError::NoAnswer));
        assert!(state.site.is_running());
        state.finish_site(2, Err(WebsiteError::NoAnswer));
        assert!(matches!(state.site, Lookup::Failed(WebsiteError::NoAnswer)));
    }

    #[test]
    fn the_two_checks_do_not_overtake_each_other() {
        let mut state = WebState::new();
        let _ = state.check_site();
        let _ = state.check_email();
        state.finish_email(1, Err(DnsError::Timeout));
        assert!(
            state.site.is_running(),
            "an email answer must not end the site check"
        );
        assert!(matches!(state.email, Lookup::Failed(DnsError::Timeout)));
    }

    #[test]
    fn pages_have_titles() {
        assert_eq!(WebPage::Website.title(), "Website");
        assert_eq!(WebPage::ALL.len(), 2);
    }
}
