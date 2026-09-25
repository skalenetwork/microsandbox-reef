use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

    pub fn live(self, vm: Option<Self>) -> bool {
        self == Self::Running && vm == Some(Self::Running)
    }
}

impl std::str::FromStr for VmStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            other => Err(format!("invalid vm status: {other:?}")),
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

pub fn plan(desired: VmStatus, vm: Option<VmStatus>, drift: Drift) -> &'static [Action] {
    use Action::*;
    use VmStatus::{Running, Stopped};
    match (desired, vm, drift) {
        (Running, None, _) => &[Create],
        (Running, Some(_), Drift::Role) => &[Remove, Create],
        (Running, Some(Running), Drift::Env) => &[Stop, Modify, Start],
        (Running, Some(Stopped), Drift::Env) => &[Modify, Start],
        (Running, Some(Stopped), Drift::None) => &[Start],
        (Running, Some(Running), Drift::None) => &[],
        (Stopped, Some(_), Drift::Role) => &[Remove],
        (Stopped, Some(Running), Drift::Env) => &[Stop, Modify],
        (Stopped, Some(Stopped), Drift::Env) => &[Modify],
        (Stopped, Some(Running), Drift::None) => &[Stop],
        (Stopped, _, _) => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;
    use VmStatus::{Running, Stopped};

    #[test]
    fn every_case() {
        let (up, down) = (Some(Running), Some(Stopped));
        let cases: &[(VmStatus, Option<VmStatus>, Drift, &[Action])] = &[
            (Running, None, Drift::None, &[Create]),
            (Running, None, Drift::Env, &[Create]),
            (Running, None, Drift::Role, &[Create]),
            (Running, up, Drift::None, &[]),
            (Running, up, Drift::Env, &[Stop, Modify, Start]),
            (Running, up, Drift::Role, &[Remove, Create]),
            (Running, down, Drift::None, &[Start]),
            (Running, down, Drift::Env, &[Modify, Start]),
            (Running, down, Drift::Role, &[Remove, Create]),
            (Stopped, None, Drift::None, &[]),
            (Stopped, None, Drift::Env, &[]),
            (Stopped, None, Drift::Role, &[]),
            (Stopped, up, Drift::None, &[Stop]),
            (Stopped, up, Drift::Env, &[Stop, Modify]),
            (Stopped, up, Drift::Role, &[Remove]),
            (Stopped, down, Drift::None, &[]),
            (Stopped, down, Drift::Env, &[Modify]),
            (Stopped, down, Drift::Role, &[Remove]),
        ];
        for &(desired, vm, drift, expected) in cases {
            assert_eq!(
                plan(desired, vm, drift),
                expected,
                "{desired:?} {vm:?} {drift:?}"
            );
        }
    }

    #[test]
    fn only_an_agent_meant_to_run_with_its_vm_up_is_live() {
        assert!(Running.live(Some(Running)));
        assert!(!Running.live(Some(Stopped)));
        assert!(!Running.live(None));
        assert!(!Stopped.live(Some(Running)));
        assert!(!Stopped.live(None));
    }
}
