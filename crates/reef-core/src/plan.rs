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
pub enum Drift {
    None,
    Env,
    Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub desired: Desired,
    pub drift: Drift,
    pub vm: Option<VmStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Modify,
    Start,
    Stop,
    Remove,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Modify => "modify",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Remove => "remove",
        }
    }
}

pub fn plan(facts: Facts) -> &'static [Action] {
    use Action::*;
    match (facts.desired, facts.vm, facts.drift) {
        (Desired::Running, None, _) => &[Create],
        (Desired::Running, Some(_), Drift::Role) => &[Remove, Create],
        (Desired::Running, Some(VmStatus::Running), Drift::Env) => &[Stop, Modify, Start],
        (Desired::Running, Some(VmStatus::Stopped), Drift::Env) => &[Modify, Start],
        (Desired::Running, Some(VmStatus::Stopped), Drift::None) => &[Start],
        (Desired::Running, Some(VmStatus::Running), Drift::None) => &[],
        (Desired::Stopped, Some(VmStatus::Running), Drift::Env) => &[Stop, Modify],
        (Desired::Stopped, Some(VmStatus::Stopped), Drift::Env) => &[Modify],
        (Desired::Stopped, Some(VmStatus::Running), _) => &[Stop],
        (Desired::Stopped, _, _) => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    fn facts(desired: Desired, vm: Option<VmStatus>, drift: Drift) -> Facts {
        Facts { desired, drift, vm }
    }

    #[test]
    fn every_case() {
        let running = Desired::Running;
        let stopped = Desired::Stopped;
        let up = Some(VmStatus::Running);
        let down = Some(VmStatus::Stopped);
        let cases: &[(Facts, &[Action])] = &[
            (facts(running, None, Drift::None), &[Create]),
            (facts(running, None, Drift::Env), &[Create]),
            (facts(running, None, Drift::Role), &[Create]),
            (facts(running, up, Drift::None), &[]),
            (facts(running, up, Drift::Env), &[Stop, Modify, Start]),
            (facts(running, up, Drift::Role), &[Remove, Create]),
            (facts(running, down, Drift::None), &[Start]),
            (facts(running, down, Drift::Env), &[Modify, Start]),
            (facts(running, down, Drift::Role), &[Remove, Create]),
            (facts(stopped, None, Drift::None), &[]),
            (facts(stopped, None, Drift::Env), &[]),
            (facts(stopped, None, Drift::Role), &[]),
            (facts(stopped, up, Drift::None), &[Stop]),
            (facts(stopped, up, Drift::Env), &[Stop, Modify]),
            (facts(stopped, up, Drift::Role), &[Stop]),
            (facts(stopped, down, Drift::None), &[]),
            (facts(stopped, down, Drift::Env), &[Modify]),
            (facts(stopped, down, Drift::Role), &[]),
        ];
        for (input, expected) in cases {
            assert_eq!(plan(*input), *expected, "{input:?}");
        }
    }
}
