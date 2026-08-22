use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use aws_sdk_cloudwatchlogs::Client as Logs;
use aws_sdk_ecs::Client as Ecs;
use aws_sdk_ecs::types::{
    AssignPublicIp, AwsVpcConfiguration, ContainerOverride, KeyValuePair, LaunchType,
    NetworkConfiguration, TaskOverride,
};
use aws_sdk_s3::Client as S3;
use aws_sdk_s3::primitives::ByteStream;
use tokio::runtime::Runtime;

use crate::Fargate;

pub struct Aws {
    runtime: Runtime,
    s3: S3,
    ecs: Ecs,
    logs: Logs,
}

pub enum Log {
    Line(String),
    Stopped,
}

impl Aws {
    pub fn new() -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|err| format!("cannot talk to aws: {err}"))?;
        let config = runtime.block_on(aws_config::load_from_env());

        Ok(Self {
            s3: S3::new(&config),
            ecs: Ecs::new(&config),
            logs: Logs::new(&config),
            runtime,
        })
    }

    pub fn put_job(&self, at: &Fargate, key: &str, job: &str) -> Result<String, String> {
        self.runtime.block_on(async {
            self.s3
                .put_object()
                .bucket(&at.bucket)
                .key(key)
                .body(ByteStream::from(job.as_bytes().to_vec()))
                .send()
                .await
                .map_err(|err| format!("cannot put the job in s3: {err}"))
        })?;

        Ok(format!("s3://{}/{key}", at.bucket))
    }

    pub fn run_task(&self, at: &Fargate, job_uri: &str) -> Result<String, String> {
        let overrides = TaskOverride::builder()
            .container_overrides(
                ContainerOverride::builder()
                    .name(&at.container)
                    .environment(
                        KeyValuePair::builder()
                            .name("JOB_URI")
                            .value(job_uri)
                            .build(),
                    )
                    .build(),
            )
            .build();

        let network = NetworkConfiguration::builder()
            .awsvpc_configuration(
                AwsVpcConfiguration::builder()
                    .set_subnets(Some(at.subnets.clone()))
                    .set_security_groups(Some(at.security_groups.clone()))
                    .assign_public_ip(AssignPublicIp::Enabled)
                    .build()
                    .map_err(|err| format!("the network is not one: {err}"))?,
            )
            .build();

        let started = self.runtime.block_on(async {
            self.ecs
                .run_task()
                .cluster(&at.cluster)
                .task_definition(&at.task_definition)
                .launch_type(LaunchType::Fargate)
                .overrides(overrides)
                .network_configuration(network)
                .send()
                .await
                .map_err(|err| format!("cannot start the task: {err}"))
        })?;

        started
            .tasks()
            .first()
            .and_then(|task| task.task_arn())
            .map(ToOwned::to_owned)
            .ok_or_else(|| "the task started without an arn".to_owned())
    }

    pub fn lines(&self, at: &Fargate, stream: &str, task: &str) -> Receiver<Result<Log, String>> {
        let (send, receive) = mpsc::channel();
        let (logs, ecs) = (self.logs.clone(), self.ecs.clone());
        let group = at.log_group.clone();
        let cluster = at.cluster.clone();
        let stream = stream.to_owned();
        let task = task.to_owned();

        self.runtime.spawn(async move {
            let mut after: Option<String> = None;

            loop {
                let answered = logs
                    .get_log_events()
                    .log_group_name(&group)
                    .log_stream_name(&stream)
                    .start_from_head(true)
                    .set_next_token(after.clone())
                    .send()
                    .await;

                let quiet = match answered {
                    Ok(answered) => {
                        let events = answered.events();
                        let quiet = events.is_empty();

                        for message in events.iter().filter_map(|event| event.message()) {
                            if send.send(Ok(Log::Line(message.to_owned()))).is_err() {
                                return;
                            }
                        }
                        if let Some(next) = answered.next_forward_token() {
                            after = Some(next.to_owned());
                        }
                        quiet
                    }
                    Err(err) => {
                        if after.is_some() {
                            let _ = send.send(Err(format!("cannot read the task's logs: {err}")));
                            return;
                        }
                        true
                    }
                };

                if quiet && stopped(&ecs, &cluster, &task).await {
                    let _ = send.send(Ok(Log::Stopped));
                    return;
                }
                tokio::time::sleep(POLL_EVERY).await;
            }
        });

        receive
    }
}

async fn stopped(ecs: &Ecs, cluster: &str, task: &str) -> bool {
    let described = ecs
        .describe_tasks()
        .cluster(cluster)
        .tasks(task)
        .send()
        .await;

    described.is_ok_and(|described| {
        described
            .tasks()
            .first()
            .and_then(|task| task.last_status())
            .is_some_and(|status| status == "STOPPED")
    })
}

const POLL_EVERY: Duration = Duration::from_secs(2);
