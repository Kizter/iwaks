//! Cover-art providers and the fallback chain.
//!
//! Every provider answers the same question — "which URL can Discord's servers
//! fetch for this artist/album?" — and the chain tries them in order until one
//! hits. The distinction that matters throughout is between a *settled miss*
//! (`Ok(None)`: the provider answered and has no match) and an *unanswered*
//! question (`Err`: the provider could not be reached). Only the former may be
//! cached as "this album has no cover".

use crate::{ytmusic, CoverError};

/// One album-art source.
pub trait CoverProvider {
    /// Stable name for logs and tests.
    fn name(&self) -> &'static str;

    /// `Ok(Some(url))` found, `Ok(None)` a settled miss, `Err` retryable.
    fn lookup(&self, artist: &str, album: &str) -> Result<Option<String>, CoverError>;
}

/// Apple's iTunes Search API — the default provider.
#[derive(Debug, Default, Clone, Copy)]
pub struct Itunes;

impl CoverProvider for Itunes {
    fn name(&self) -> &'static str {
        "iTunes"
    }

    fn lookup(&self, artist: &str, album: &str) -> Result<Option<String>, CoverError> {
        crate::lookup(artist, album)
    }
}

/// YouTube Music — a best-effort community fallback for what iTunes lacks.
#[derive(Debug, Default, Clone, Copy)]
pub struct YtMusic;

impl CoverProvider for YtMusic {
    fn name(&self) -> &'static str {
        "YouTube Music"
    }

    fn lookup(&self, artist: &str, album: &str) -> Result<Option<String>, CoverError> {
        ytmusic::lookup(artist, album)
    }
}

/// Tries its providers in order; the first hit wins.
#[derive(Default)]
pub struct Fallback {
    providers: Vec<Box<dyn CoverProvider>>,
}

impl Fallback {
    /// Build a chain from an ordered list of providers.
    pub fn new(providers: Vec<Box<dyn CoverProvider>>) -> Self {
        Self { providers }
    }
}

impl CoverProvider for Fallback {
    fn name(&self) -> &'static str {
        "fallback"
    }

    fn lookup(&self, artist: &str, album: &str) -> Result<Option<String>, CoverError> {
        let mut failure = None;
        for provider in &self.providers {
            match provider.lookup(artist, album) {
                Ok(Some(url)) => return Ok(Some(url)),
                Ok(None) => {}
                Err(err) => failure = Some(err),
            }
        }
        // If any provider could not be reached, the question is not settled —
        // returning `Ok(None)` would cache a miss that a retry might answer.
        failure.map_or(Ok(None), Err)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    /// Scripted answer plus a shared counter of how many providers ran.
    struct Fake {
        answer: Result<Option<String>, ()>,
        calls: Rc<Cell<usize>>,
    }

    impl Fake {
        fn provider(
            calls: &Rc<Cell<usize>>,
            answer: Result<Option<String>, ()>,
        ) -> Box<dyn CoverProvider> {
            Box::new(Self {
                answer,
                calls: Rc::clone(calls),
            })
        }
    }

    impl CoverProvider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn lookup(&self, _artist: &str, _album: &str) -> Result<Option<String>, CoverError> {
            self.calls.set(self.calls.get() + 1);
            match &self.answer {
                Ok(found) => Ok(found.clone()),
                Err(()) => Err(CoverError::Request("down".into())),
            }
        }
    }

    fn hit(url: &str) -> Result<Option<String>, ()> {
        Ok(Some(url.to_string()))
    }

    fn chain(calls: &Rc<Cell<usize>>, answers: &[Result<Option<String>, ()>]) -> Fallback {
        Fallback::new(
            answers
                .iter()
                .map(|a| Fake::provider(calls, a.clone()))
                .collect(),
        )
    }

    #[test]
    fn first_hit_wins_and_later_providers_are_not_consulted() {
        let calls = Rc::new(Cell::new(0));
        let chain = chain(&calls, &[hit("https://a"), hit("https://b")]);
        assert_eq!(
            chain.lookup("A", "B").expect("answered").as_deref(),
            Some("https://a")
        );
        assert_eq!(calls.get(), 1, "chain must stop at the first hit");
    }

    #[test]
    fn a_miss_falls_through_to_the_next_provider() {
        let calls = Rc::new(Cell::new(0));
        let chain = chain(&calls, &[Ok(None), hit("https://b")]);
        assert_eq!(
            chain.lookup("A", "B").expect("answered").as_deref(),
            Some("https://b")
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn all_misses_are_a_settled_miss() {
        let calls = Rc::new(Cell::new(0));
        let chain = chain(&calls, &[Ok(None), Ok(None)]);
        assert_eq!(chain.lookup("A", "B").expect("answered"), None);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn a_failure_keeps_the_question_retryable() {
        // One provider is down, the other says no. The down provider might have
        // matched, so this must not become a cached miss.
        let calls = Rc::new(Cell::new(0));
        let chain = chain(&calls, &[Ok(None), Err(())]);
        assert!(matches!(
            chain.lookup("A", "B"),
            Err(CoverError::Request(_))
        ));
    }

    #[test]
    fn a_hit_after_a_failure_is_returned() {
        let calls = Rc::new(Cell::new(0));
        let chain = chain(&calls, &[Err(()), hit("https://b")]);
        assert_eq!(
            chain.lookup("A", "B").expect("answered").as_deref(),
            Some("https://b")
        );
    }

    #[test]
    fn the_default_chain_tries_itunes_first() {
        let chain = crate::default_chain();
        let names: Vec<_> = chain.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["iTunes", "YouTube Music"]);
    }
}
