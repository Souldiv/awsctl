use crate::aws::ec2::TaggedInstance;

pub const BASTION_COMPONENT: &str = "bastion";
const JUMP_HOST_SUFFIX: &str = "-jump-host";
const BASTION_HOST_NAME: &str = "bastion-host";

pub fn fallback_name_tag_patterns() -> Vec<String> {
    vec![
        format!("*{}", JUMP_HOST_SUFFIX),
        String::from(BASTION_HOST_NAME),
    ]
}

pub fn best_bastions(
    instances: &[TaggedInstance],
    account_label: Option<&str>,
) -> Vec<TaggedInstance> {
    let ranked: Vec<(u8, &TaggedInstance)> = instances
        .iter()
        .filter_map(|instance| bastion_rank(instance, account_label).map(|rank| (rank, instance)))
        .collect();

    let Some(best_rank) = ranked.iter().map(|(rank, _)| *rank).min() else {
        return vec![];
    };

    let mut best: Vec<TaggedInstance> = ranked
        .into_iter()
        .filter(|(rank, _)| *rank == best_rank)
        .map(|(_, instance)| instance.clone())
        .collect();
    best.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
    best
}

fn bastion_rank(instance: &TaggedInstance, account_label: Option<&str>) -> Option<u8> {
    if instance.component.as_deref() == Some(BASTION_COMPONENT) {
        return Some(0);
    }

    let name = instance.name.as_deref()?;
    let labelled_jump_host = account_label.map(|label| format!("{}{}", label, JUMP_HOST_SUFFIX));

    if labelled_jump_host.as_deref() == Some(name) {
        Some(1)
    } else if name.ends_with(JUMP_HOST_SUFFIX) {
        Some(2)
    } else if name == BASTION_HOST_NAME {
        Some(3)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{best_bastions, fallback_name_tag_patterns};
    use crate::aws::ec2::TaggedInstance;

    fn instance(instance_id: &str, name: Option<&str>, component: Option<&str>) -> TaggedInstance {
        TaggedInstance {
            instance_id: String::from(instance_id),
            name: name.map(String::from),
            component: component.map(String::from),
        }
    }

    fn ids(instances: &[TaggedInstance]) -> Vec<&str> {
        instances
            .iter()
            .map(|instance| instance.instance_id.as_str())
            .collect()
    }

    #[test]
    fn prefers_the_component_tag_over_any_name() {
        let found = [
            instance("i-2", Some("acme-jump-host"), None),
            instance("i-1", Some("anything"), Some("bastion")),
        ];

        assert_eq!(ids(&best_bastions(&found, Some("acme"))), ["i-1"]);
    }

    #[test]
    fn prefers_the_labelled_jump_host_over_other_jump_hosts() {
        let found = [
            instance("i-1", Some("other-jump-host"), None),
            instance("i-2", Some("acme-jump-host"), None),
        ];

        assert_eq!(ids(&best_bastions(&found, Some("acme"))), ["i-2"]);
    }

    #[test]
    fn takes_any_jump_host_when_no_label_is_given() {
        let found = [
            instance("i-1", Some("bastion-host"), None),
            instance("i-2", Some("acme-jump-host"), None),
        ];

        assert_eq!(ids(&best_bastions(&found, None)), ["i-2"]);
    }

    #[test]
    fn falls_back_to_bastion_host() {
        let found = [instance("i-1", Some("bastion-host"), None)];

        assert_eq!(ids(&best_bastions(&found, Some("acme"))), ["i-1"]);
    }

    #[test]
    fn returns_every_tie_ordered_by_instance_id() {
        let found = [
            instance("i-b", Some("one-jump-host"), None),
            instance("i-a", Some("two-jump-host"), None),
        ];

        assert_eq!(ids(&best_bastions(&found, None)), ["i-a", "i-b"]);
    }

    #[test]
    fn ignores_instances_that_match_no_rule() {
        let found = [
            instance("i-1", Some("worker-node"), None),
            instance("i-2", None, Some("web")),
        ];

        assert!(best_bastions(&found, None).is_empty());
    }

    #[test]
    fn searches_for_jump_hosts_and_bastion_host_by_name() {
        assert_eq!(
            fallback_name_tag_patterns(),
            ["*-jump-host", "bastion-host"]
        );
    }
}
