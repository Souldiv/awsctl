use anyhow::Result;
use aws_config::SdkConfig;
use aws_sdk_ec2::types::Filter;
use aws_sdk_ec2::Client;
use aws_smithy_types_convert::date_time::DateTimeExt;
use chrono::{DateTime, Utc};
use inquire::Confirm;
use kube::api::{ListParams, Patch, PatchParams};
use kube::{api::Api as KubeApi, Client as KubeClient};
use kube_derive::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::{sleep, Duration};

pub struct SnapshotProvider {
    client: Client,
}

/// Converts a k8s-openapi `jiff::Timestamp` into a `chrono::DateTime<Utc>`.
fn jiff_to_chrono(ts: k8s_openapi::jiff::Timestamp) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(ts.as_nanosecond() as i64)
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub aws_volume_snapshot_id: Option<String>,

    pub eks_volume_snapshot_id: Option<String>,
    pub eks_volume_snapshot_content_id: Option<String>,

    pub cnpg_backup_id: Option<String>,

    pub namespace: String,
    pub created_at: DateTime<Utc>,
}

// Define Backup
#[derive(CustomResource, Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[kube(
    group = "postgresql.cnpg.io",
    version = "v1",
    kind = "Backup",
    plural = "backups",
    namespaced = true
)]
pub struct BackupSpec {
    pub method: String,
}

// Define VolumeSnapshot
#[derive(CustomResource, Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[kube(
    group = "snapshot.storage.k8s.io",
    version = "v1",
    kind = "VolumeSnapshot",
    plural = "volumesnapshots",
    namespaced = true
)]
pub struct VolumeSnapshotSpec {}

// Define VolumeSnapshotContent
#[derive(CustomResource, Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[kube(
    group = "snapshot.storage.k8s.io",
    version = "v1",
    kind = "VolumeSnapshotContent",
    plural = "volumesnapshotcontents",
    status = "VolumeSnapshotContentStatus",
    derive = "PartialEq"
)]
#[allow(non_snake_case)]
pub struct VolumeSnapshotContentSpec {
    source: Source,
    volumeSnapshotRef: VolumeSnapshotRef,
}

// Define the VolumeSnapshotContentStatus struct
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[allow(non_snake_case)]
pub struct VolumeSnapshotContentStatus {
    snapshotHandle: Option<String>,
}

// Define the Source struct
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[allow(non_snake_case)]
pub struct Source {
    volumeHandle: Option<String>, // AWS Volume ID
}

// Define the VolumeSnapshotRef struct
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct VolumeSnapshotRef {
    name: Option<String>,
    namespace: Option<String>,
}

impl SnapshotProvider {
    pub fn new(config: &SdkConfig) -> Self {
        let client = Client::new(config);

        Self { client }
    }

    pub async fn list_snapshots(&self, namespace: String) -> Result<Vec<Snapshot>> {
        let filter = Filter::builder()
            .name("tag:kubernetes.io/created-for/pvc/namespace")
            .values(namespace.clone())
            .build();
        let volumes_response = self
            .client
            .describe_volumes()
            .filters(filter)
            .send()
            .await?;

        let volumes: Vec<String> = volumes_response
            .volumes()
            .iter()
            .map(|volume| volume.volume_id().unwrap().to_string())
            .collect();

        let snapshot_filter = Filter::builder()
            .name("volume-id")
            .set_values(Some(volumes))
            .build();
        let snapshots_response = self
            .client
            .describe_snapshots()
            .owner_ids("self")
            .filters(snapshot_filter)
            .send()
            .await?;

        let list_of_snapshots: Vec<Snapshot> = snapshots_response
            .snapshots()
            .iter()
            .map(|snapshot| Snapshot {
                aws_volume_snapshot_id: Some(snapshot.snapshot_id().unwrap().to_string()),
                eks_volume_snapshot_id: None,
                eks_volume_snapshot_content_id: None,
                cnpg_backup_id: None,
                namespace: namespace.clone(),
                created_at: snapshot.start_time().unwrap().to_chrono_utc().unwrap(),
            })
            .collect();

        Ok(list_of_snapshots)
    }

    pub async fn list_volume_snapshots(&self, namespace: String) -> Result<Vec<Snapshot>> {
        let kube_client = KubeClient::try_default().await?;
        let volumesnapshotcontents: KubeApi<VolumeSnapshotContent> = KubeApi::all(kube_client);

        let contents = volumesnapshotcontents.list(&ListParams::default()).await?;

        // // Collect VolumeSnapshots and map them to VolumeSnapshot structs
        let mut result: Vec<Snapshot> = Vec::new();

        for content in contents.items {
            let eks_volume_snapshot_content_id = content.metadata.name.clone().unwrap_or_default();

            let eks_volume_snapshot_id = content
                .spec
                .volumeSnapshotRef
                .name
                .clone()
                .unwrap_or_default();
            let namespace = content
                .spec
                .volumeSnapshotRef
                .namespace
                .clone()
                .unwrap_or_default();

            let aws_volume_snapshot_id = content
                .status
                .as_ref()
                .and_then(|status| status.snapshotHandle.clone())
                .unwrap_or_default();
            let created_at = content
                .metadata
                .creation_timestamp
                .clone()
                .map(|time| jiff_to_chrono(time.0))
                .unwrap_or_else(Utc::now);

            result.push(Snapshot {
                eks_volume_snapshot_id: Some(eks_volume_snapshot_id),
                eks_volume_snapshot_content_id: Some(eks_volume_snapshot_content_id),
                aws_volume_snapshot_id: Some(aws_volume_snapshot_id),
                cnpg_backup_id: None,
                namespace,
                created_at,
            });
        }

        // filter out the ones that are not in the namespace
        let filtered_result = result
            .into_iter()
            .filter(|snapshot| snapshot.namespace == namespace.clone())
            .collect();

        Ok(filtered_result)
    }

    pub async fn list_cnpg_backups(&self, namespace: String) -> Result<Vec<Snapshot>> {
        let kube_client = KubeClient::try_default().await?;
        let backups: KubeApi<Backup> = KubeApi::namespaced(kube_client, &namespace);

        let backups = backups.list(&ListParams::default()).await?;

        let list_of_backups: Vec<Snapshot> = backups
            .items
            .iter()
            .map(|backup| Snapshot {
                aws_volume_snapshot_id: None,
                eks_volume_snapshot_id: None,
                eks_volume_snapshot_content_id: None,
                cnpg_backup_id: Some(backup.metadata.name.clone().unwrap_or_default()),
                namespace: backup.metadata.namespace.clone().unwrap_or_default(),
                created_at: jiff_to_chrono(backup.metadata.creation_timestamp.clone().unwrap().0),
            })
            .collect();

        let filtered_result = list_of_backups
            .into_iter()
            .filter(|backup| backup.namespace == namespace.clone())
            .collect();

        Ok(filtered_result)
    }

    pub async fn delete_aws_snapshots(&self, snapshots: Vec<Snapshot>, yes: bool) -> Result<()> {
        let allow_delete = yes
            || Confirm::new(
                format!(
                    "Are you sure you want to delete {} snapshots? ",
                    snapshots.len()
                )
                .as_str(),
            )
            .with_default(false)
            .prompt()?;

        match allow_delete {
            true => {
                let mut remaining = snapshots.len();
                for snapshot in snapshots {
                    // try to delete the snapshot and retry if it fails
                    match self
                        .client
                        .delete_snapshot()
                        .snapshot_id(snapshot.aws_volume_snapshot_id.as_deref().unwrap_or(""))
                        .send()
                        .await
                    {
                        Ok(_) => {
                            remaining -= 1;
                            println!(
                                "Deleted snapshot: {} remaining: {}",
                                snapshot.aws_volume_snapshot_id.as_deref().unwrap_or(""),
                                remaining
                            );
                        }
                        Err(e) => {
                            println!(
                                "Error deleting snapshot: {} error: {}",
                                snapshot.aws_volume_snapshot_id.as_deref().unwrap_or(""),
                                e
                            );
                            sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
            false => {
                println!("Aborting deletion");
            }
        }

        Ok(())
    }

    pub async fn delete_eks_snapshots(
        &self,
        volume_snapshots: Vec<Snapshot>,
        yes: bool,
    ) -> Result<()> {
        let allow_delete = yes
            || Confirm::new(
                format!(
                    "Are you sure you want to delete {} volume snapshots?",
                    volume_snapshots.len()
                )
                .as_str(),
            )
            .with_default(false)
            .prompt()?;

        let kube_client = KubeClient::try_default().await?;

        let volumesnapshotcontents: KubeApi<VolumeSnapshotContent> =
            KubeApi::all(kube_client.clone());

        if allow_delete {
            let mut remaining = volume_snapshots.len();
            for volume_snapshot in volume_snapshots {
                let volumesnapshots: KubeApi<VolumeSnapshot> =
                    KubeApi::namespaced(kube_client.clone(), &volume_snapshot.namespace);

                let mut success = true; // Initialize success to true

                match volumesnapshotcontents
                    .delete(
                        &volume_snapshot
                            .eks_volume_snapshot_content_id
                            .as_deref()
                            .unwrap_or(""),
                        &Default::default(),
                    )
                    .await
                {
                    Ok(_) => {
                        println!(
                            "Deleted volume snapshot content: {}, remaining: {}",
                            volume_snapshot
                                .eks_volume_snapshot_content_id
                                .as_deref()
                                .unwrap_or(""),
                            remaining
                        );
                    }
                    Err(e) => {
                        println!(
                            "Error deleting volume snapshot content: {} error: {}",
                            volume_snapshot
                                .eks_volume_snapshot_content_id
                                .as_deref()
                                .unwrap_or(""),
                            e
                        );
                        success = false;
                    }
                }
                match volumesnapshots
                    .delete(
                        &volume_snapshot
                            .eks_volume_snapshot_id
                            .as_deref()
                            .unwrap_or(""),
                        &Default::default(),
                    )
                    .await
                {
                    Ok(_) => {
                        println!(
                            "Deleted volume snapshot: {}, remaining: {}",
                            volume_snapshot
                                .eks_volume_snapshot_id
                                .as_deref()
                                .unwrap_or(""),
                            remaining
                        );
                    }
                    Err(e) => {
                        println!(
                            "Error deleting volume snapshot: {} error: {}",
                            volume_snapshot
                                .eks_volume_snapshot_id
                                .as_deref()
                                .unwrap_or(""),
                            e
                        );
                        success = false;
                    }
                }
                if success {
                    remaining -= 1;

                    let patch = json!({
                      "metadata": {
                          "finalizers": []
                      }
                    });
                    // update finalizer to be empty
                    let _ = volumesnapshotcontents
                        .patch(
                            &volume_snapshot
                                .eks_volume_snapshot_content_id
                                .as_deref()
                                .unwrap_or(""),
                            &PatchParams::default(),
                            &Patch::Merge(&patch),
                        )
                        .await;

                    let _ = volumesnapshots
                        .patch(
                            &volume_snapshot
                                .eks_volume_snapshot_id
                                .as_deref()
                                .unwrap_or(""),
                            &PatchParams::default(),
                            &Patch::Merge(&patch),
                        )
                        .await;
                }
            }
        }

        Ok(())
    }

    pub async fn delete_cnpg_backups(&self, backups: Vec<Snapshot>, yes: bool) -> Result<()> {
        let allow_delete = yes
            || Confirm::new(
                format!(
                    "Are you sure you want to delete {} backups? ",
                    backups.len()
                )
                .as_str(),
            )
            .with_default(false)
            .prompt()?;

        let kube_client = KubeClient::try_default().await?;

        if allow_delete {
            let mut remaining = backups.len();
            for backup in backups {
                let cnpgbackups: KubeApi<Backup> =
                    KubeApi::namespaced(kube_client.clone(), &backup.namespace);

                match cnpgbackups
                    .delete(
                        &backup.cnpg_backup_id.as_deref().unwrap_or(""),
                        &Default::default(),
                    )
                    .await
                {
                    Ok(_) => {
                        remaining -= 1;
                        println!(
                            "Deleted cnpg backup: {}, remaining: {}",
                            backup.cnpg_backup_id.as_deref().unwrap_or(""),
                            remaining
                        );
                    }
                    Err(e) => {
                        println!("Error deleting cnpg backup: {:?}", e);
                    }
                }

                let patch = json!({
                  "metadata": {
                      "finalizers": []
                  }
                });
                // update finalizer to be empty
                let _ = cnpgbackups
                    .patch(
                        &backup.cnpg_backup_id.as_deref().unwrap_or(""),
                        &PatchParams::default(),
                        &Patch::Merge(&patch),
                    )
                    .await;
            }
        }

        Ok(())
    }
}
