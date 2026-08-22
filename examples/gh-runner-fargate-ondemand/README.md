# gh-runner-fargate-ondemand

A self-hosted runner that runs nothing itself: every job becomes one AWS Fargate task, which lives only as long as the job. The task runs the
[job-runner](../job-runner) image, reads the job from S3 and streams its output back
through CloudWatch.

Create the infrastructure first — an S3 bucket, a log group, an ECS cluster and the task
definition pointing at your published image:

```sh
cd terraform
terraform init
terraform apply -var image=YOUR_REGISTRY/job-runner:TAG
```

`terraform output run` prints the command with every name filled in. Register a runner, then
dispatch to it:

```sh
cargo run -p gh-runner-registration -- --url https://github.com/OWNER/REPO --token AAAA...
cargo run -p gh-runner-fargate-ondemand -- \
  --cluster CLUSTER --task-definition FAMILY --bucket BUCKET \
  --log-group GROUP --subnets SUBNET_IDS --security-groups SG_ID
```

Credentials come from the environment the AWS CLI uses. The workflow is the same as for
[gh-runner-local](../gh-runner-local): `runs-on: self-hosted`.
