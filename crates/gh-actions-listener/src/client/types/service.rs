use serde::{Deserialize, Serialize};

pub const ACTIONS: &str = "a85b8835-c1a1-4aac-ae97-1c3d0ba72dbd";

const MONIKER: &str = "PublicAccessMapping";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionData {
    pub authenticated_user: Identity,
    pub authorized_user: Identity,
    pub instance_id: String,
    pub deployment_id: String,
    pub deployment_type: String,
    pub location_service_data: LocationServiceData,
}

impl ConnectionData {
    /// A service reachable at one address, which is all a runner needs to hear.
    pub fn at(base: &str) -> Self {
        // Named, since an identity without an id at all is not one the runner will read.
        let user = Identity {
            id: "00000000-0000-0000-0000-000000000001".to_owned(),
            provider_display_name: "canopy".to_owned(),
            properties: serde_json::json!({}),
        };

        Self {
            authenticated_user: user.clone(),
            authorized_user: user,
            instance_id: "00000000-0000-0000-0000-000000000002".to_owned(),
            deployment_id: "00000000-0000-0000-0000-000000000003".to_owned(),
            deployment_type: "hosted".to_owned(),
            location_service_data: LocationServiceData {
                service_owner: ACTIONS.to_owned(),
                default_access_mapping_moniker: MONIKER.to_owned(),
                last_change_id: 1,
                last_change_id64: 1,
                client_cache_fresh: true,
                access_mappings: vec![AccessMapping {
                    display_name: "Public".to_owned(),
                    moniker: MONIKER.to_owned(),
                    access_point: base.to_owned(),
                    virtual_directory: String::new(),
                }],
                service_definitions: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Identity {
    pub id: String,
    pub provider_display_name: String,
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationServiceData {
    pub service_owner: String,
    pub default_access_mapping_moniker: String,
    pub last_change_id: i64,
    pub last_change_id64: i64,
    pub client_cache_fresh: bool,
    pub access_mappings: Vec<AccessMapping>,
    pub service_definitions: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessMapping {
    pub display_name: String,
    pub moniker: String,
    pub access_point: String,
    pub virtual_directory: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLocation {
    pub id: String,
    pub area: String,
    pub resource_name: String,
    pub route_template: String,
    pub resource_version: u32,
    pub min_version: f32,
    pub max_version: f32,
    pub released_version: String,
}

impl ResourceLocation {
    pub fn new(id: &str, resource: &str, route: &str) -> Self {
        Self {
            id: id.to_owned(),
            area: "distributedtask".to_owned(),
            resource_name: resource.to_owned(),
            route_template: route.to_owned(),
            resource_version: 1,
            min_version: 1.0,
            max_version: 7.0,
            released_version: "7.0".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    pub id: String,
    pub parent_id: Option<String>,
    /// `Job` for the job itself, `Task` for a step of it.
    pub r#type: String,
    pub name: String,
    /// What the message called it, which is how a step is told from the runner's own.
    pub ref_name: String,
    /// Where it sits among the steps, counting the ones the runner adds.
    pub order: u64,
    pub state: String,
    pub result: Option<String>,
    /// Where everything it printed was uploaded, once it has stopped printing.
    pub log: Option<LogReference>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LogReference {
    pub id: i64,
}

impl Record {
    pub fn is_step(&self) -> bool {
        self.r#type == "Task"
    }

    pub fn finished(&self) -> bool {
        self.state == "completed"
    }
}

/// The actions a job uses, which a runner asks where to get before it starts one.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionReferences {
    pub actions: Vec<ActionReference>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionReference {
    pub name_with_owner: String,
    pub r#ref: String,
}

impl ActionReference {
    /// How a runner looks the answer back up.
    pub fn key(&self) -> String {
        format!("{}@{}", self.name_with_owner, self.r#ref)
    }
}

/// Where each of them is, under the name it was asked for by.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionDownloads {
    pub actions: std::collections::BTreeMap<String, ActionDownload>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionDownload {
    pub name_with_owner: String,
    pub r#ref: String,
    pub resolved_name_with_owner: String,
    /// What the unpacked copy is named after, so two refs of one action stay apart.
    pub resolved_sha: String,
    pub tarball_url: String,
    pub zipball_url: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lines {
    pub value: Vec<String>,
    pub step_id: String,
}
