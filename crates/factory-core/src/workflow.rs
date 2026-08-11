use factory_models::TaskState;

pub struct Workflow;

impl Workflow {
    pub fn initial_state(has_dependencies: bool) -> TaskState {
        if has_dependencies {
            TaskState::Pending
        } else {
            TaskState::Ready
        }
    }

    pub fn can_transition(from: TaskState, to: TaskState) -> bool {
        if from == to {
            return true;
        }
        matches!(
            (from, to),
            (TaskState::Pending, TaskState::Ready | TaskState::Blocked)
                | (TaskState::Ready, TaskState::Running)
                | (
                    TaskState::Running,
                    TaskState::Completed | TaskState::Failed | TaskState::Blocked
                )
                | (TaskState::Blocked, TaskState::Ready)
                | (TaskState::Failed, TaskState::Ready)
        )
    }

    pub fn allowed_targets(from: TaskState) -> Vec<TaskState> {
        match from {
            TaskState::Pending => vec![TaskState::Ready, TaskState::Blocked],
            TaskState::Ready => vec![TaskState::Running],
            TaskState::Running => vec![TaskState::Completed, TaskState::Failed, TaskState::Blocked],
            TaskState::Blocked => vec![TaskState::Ready],
            TaskState::Failed => vec![TaskState::Ready],
            TaskState::Completed => vec![],
        }
    }

    pub fn next_state_for_dependent(dep_states: &[TaskState]) -> TaskState {
        if dep_states
            .iter()
            .any(|s| matches!(s, TaskState::Failed | TaskState::Blocked))
        {
            TaskState::Blocked
        } else if dep_states.iter().all(|s| *s == TaskState::Completed) {
            TaskState::Ready
        } else {
            TaskState::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_models::TaskState::*;

    #[test]
    fn initial_state_depends_on_dependencies() {
        assert_eq!(Workflow::initial_state(false), Ready);
        assert_eq!(Workflow::initial_state(true), Pending);
    }

    #[test]
    fn accepts_valid_transitions() {
        assert!(Workflow::can_transition(Pending, Ready));
        assert!(Workflow::can_transition(Pending, Blocked));
        assert!(Workflow::can_transition(Ready, Running));
        assert!(Workflow::can_transition(Running, Completed));
        assert!(Workflow::can_transition(Running, Failed));
        assert!(Workflow::can_transition(Running, Blocked));
        assert!(Workflow::can_transition(Blocked, Ready));
        assert!(Workflow::can_transition(Failed, Ready));
        assert!(Workflow::can_transition(Completed, Completed));
    }

    #[test]
    fn rejects_invalid_transitions() {
        assert!(!Workflow::can_transition(Pending, Running));
        assert!(!Workflow::can_transition(Pending, Completed));
        assert!(!Workflow::can_transition(Ready, Completed));
        assert!(!Workflow::can_transition(Running, Ready));
        assert!(!Workflow::can_transition(Blocked, Running));
        assert!(!Workflow::can_transition(Completed, Pending));
        assert!(!Workflow::can_transition(Failed, Running));
    }

    #[test]
    fn completed_is_terminal() {
        assert!(Workflow::allowed_targets(Completed).is_empty());
    }

    #[test]
    fn dependent_state_follows_dependencies() {
        assert_eq!(Workflow::next_state_for_dependent(&[Completed]), Ready);
        assert_eq!(
            Workflow::next_state_for_dependent(&[Completed, Ready]),
            Pending
        );
        assert_eq!(
            Workflow::next_state_for_dependent(&[Completed, Failed]),
            Blocked
        );
        assert_eq!(Workflow::next_state_for_dependent(&[Pending]), Pending);
        assert_eq!(
            Workflow::next_state_for_dependent(&[Blocked, Ready]),
            Blocked
        );
    }
}
