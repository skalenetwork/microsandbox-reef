use crate::name::PortName;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

pub const HOST_PORTS: RangeInclusive<u16> = 19000..=19999;

pub fn allocate_ports<'a>(
    expose: impl Iterator<Item = &'a PortName>,
    current: &BTreeMap<PortName, u16>,
    used: &BTreeSet<u16>,
) -> Result<BTreeMap<PortName, u16>, String> {
    let mut taken = used.clone();
    taken.extend(current.values());
    let mut ports = BTreeMap::new();
    for name in expose {
        let port = match current.get(name) {
            Some(port) => *port,
            None => {
                let free = HOST_PORTS
                    .clone()
                    .find(|port| !taken.contains(port))
                    .ok_or_else(|| {
                        format!(
                            "host port range {}-{} is exhausted",
                            HOST_PORTS.start(),
                            HOST_PORTS.end()
                        )
                    })?;
                taken.insert(free);
                free
            }
        };
        ports.insert(name.clone(), port);
    }
    Ok(ports)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> PortName {
        text.parse().unwrap()
    }

    #[test]
    fn keeps_existing_allocates_new_drops_removed() {
        let current = BTreeMap::from([(name("ui"), 19007)]);
        let used = BTreeSet::from([19000, 19007]);
        let expose = [name("metrics"), name("ui")];

        let ports = allocate_ports(expose.iter(), &current, &used).unwrap();
        assert_eq!(ports[&name("ui")], 19007, "existing allocation is stable");
        assert_eq!(ports[&name("metrics")], 19001, "lowest free is reused");

        let ports = allocate_ports([name("ui")].iter(), &ports, &used).unwrap();
        assert_eq!(ports.len(), 1, "removed entries are dropped");
    }

    #[test]
    fn exhaustion_is_a_named_error() {
        let used: BTreeSet<u16> = HOST_PORTS.collect();
        let err = allocate_ports([name("ui")].iter(), &BTreeMap::new(), &used).unwrap_err();
        assert!(err.contains("exhausted"), "{err}");
    }
}
