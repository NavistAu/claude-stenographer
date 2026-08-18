#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    CurrentProjectRecent,
    CurrentProjectAll,
    /// The project's subtree PLUS projects at the working dir's path
    /// ancestors (e.g. ~/ws and ~ for ~/ws/impostarr) — where sessions
    /// about this project land when run from a parent directory. A
    /// superset of CurrentProjectAll, since escalate() returns only the
    /// last tier's results.
    AncestorProjects,
    AllProjects,
}

impl Scope {
    pub fn label(&self) -> &'static str {
        match self {
            Scope::CurrentProjectRecent => "current_project_recent",
            Scope::CurrentProjectAll => "current_project_all",
            Scope::AncestorProjects => "ancestor_projects",
            Scope::AllProjects => "all_projects",
        }
    }
}

pub const LADDER: [Scope; 4] = [
    Scope::CurrentProjectRecent,
    Scope::CurrentProjectAll,
    Scope::AncestorProjects,
    Scope::AllProjects,
];

pub struct TierOutcome<T> {
    pub results: Vec<T>,
    pub sessions_searched: usize,
}

pub struct Escalated<T> {
    pub results: Vec<T>,
    pub scope_reached: Scope,
    pub sessions_searched: usize,
}

/// Run each tier in order; stop at the first tier yielding >= `target` results.
/// `run(scope)` performs the actual search for that tier.
pub fn escalate<T, F>(target: usize, mut run: F) -> Escalated<T>
where
    F: FnMut(&Scope) -> TierOutcome<T>,
{
    let mut last = TierOutcome {
        results: Vec::new(),
        sessions_searched: 0,
    };
    let mut reached = Scope::AllProjects;
    for scope in LADDER.iter() {
        reached = scope.clone();
        last = run(scope);
        if last.results.len() >= target {
            break;
        }
    }
    Escalated {
        results: last.results,
        scope_reached: reached,
        sessions_searched: last.sessions_searched,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stops_at_first_tier_with_hits() {
        let mut calls = Vec::new();
        let out = escalate(1, |scope| {
            calls.push(scope.clone());
            TierOutcome {
                results: vec![()],
                sessions_searched: 10,
            }
        });
        assert_eq!(out.scope_reached, Scope::CurrentProjectRecent);
        assert_eq!(calls, vec![Scope::CurrentProjectRecent]);
    }

    #[test]
    fn test_escalates_to_all_projects_when_empty() {
        let out: Escalated<()> = escalate(1, |scope| {
            let results = if *scope == Scope::AllProjects {
                vec![()]
            } else {
                vec![]
            };
            TierOutcome {
                results,
                sessions_searched: 100,
            }
        });
        assert_eq!(out.scope_reached, Scope::AllProjects);
        assert_eq!(out.results.len(), 1);
    }

    #[test]
    fn test_exhausts_and_reports_all_projects_on_total_miss() {
        let out: Escalated<()> = escalate(1, |_| TierOutcome {
            results: vec![],
            sessions_searched: 3900,
        });
        assert_eq!(out.scope_reached, Scope::AllProjects);
        assert!(out.results.is_empty());
        assert_eq!(out.sessions_searched, 3900);
    }
}
