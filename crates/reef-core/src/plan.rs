use crate::agent::Desired;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmStatus {
    Running,
    Stopped,
}

impl VmStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub desired: Desired,
    pub in_sync: bool,
    pub vm: Option<VmStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Start,
    Stop,
    Remove,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Remove => "remove",
        }
    }
}

pub fn plan(facts: Facts) -> &'static [Action] {
    use Action::*;
    match (facts.desired, facts.vm, facts.in_sync) {
        (Desired::Running, None, _) => &[Create],
        (Desired::Running, Some(_), false) => &[Remove, Create],
        (Desired::Running, Some(VmStatus::Stopped), true) => &[Start],
        (Desired::Running, Some(VmStatus::Running), true) => &[],
        (Desired::Stopped, Some(VmStatus::Running), _) => &[Stop],
        (Desired::Stopped, _, _) => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    fn facts(desired: Desired, vm: Option<VmStatus>, in_sync: bool) -> Facts {
        Facts {
            desired,
            in_sync,
            vm,
        }
    }

    #[test]
    fn every_case() {
        let running = Desired::Running;
        let stopped = Desired::Stopped;
        let cases: &[(Facts, &[Action])] = &[
            (facts(running, None, false), &[Create]),
            (facts(running, None, true), &[Create]),
            (facts(running, Some(VmStatus::Running), true), &[]),
            (
                facts(running, Some(VmStatus::Running), false),
                &[Remove, Create],
            ),
            (facts(running, Some(VmStatus::Stopped), true), &[Start]),
            (
                facts(running, Some(VmStatus::Stopped), false),
                &[Remove, Create],
            ),
            (facts(stopped, None, false), &[]),
            (facts(stopped, None, true), &[]),
            (facts(stopped, Some(VmStatus::Running), true), &[Stop]),
            (facts(stopped, Some(VmStatus::Running), false), &[Stop]),
            (facts(stopped, Some(VmStatus::Stopped), true), &[]),
            (facts(stopped, Some(VmStatus::Stopped), false), &[]),
        ];
        for (input, expected) in cases {
            assert_eq!(plan(*input), *expected, "{input:?}");
        }
    }
}
