terraform {
  required_providers {
    aws = { source = "hashicorp/aws" }
  }
}

provider "aws" {
  region = var.region
}

variable "region" {
  type    = string
  default = "us-east-1"
}

variable "name" {
  type    = string
  default = "canopy-job-runner"
}

variable "image" {
  type        = string
  description = "public image holding the job-runner binary"
  default     = "ferranbt/job-runner@sha256:fe6d632c63454cc999194a8df5382997e19b0eb9898cb9ad8808cbc90fba939e"
}

variable "command" {
  type    = string
  default = "aws s3 cp $JOB_URI /work/job.json && job-runner /work/job.json --json"
}

variable "cpu" {
  type    = string
  default = "1024"
}

variable "memory" {
  type    = string
  default = "2048"
}

data "aws_caller_identity" "this" {}

data "aws_vpc" "default" {
  default = true
}

data "aws_subnets" "default" {
  filter {
    name   = "vpc-id"
    values = [data.aws_vpc.default.id]
  }
}

resource "aws_s3_bucket" "jobs" {
  bucket_prefix = "${var.name}-"
  force_destroy = true
}

resource "aws_cloudwatch_log_group" "jobs" {
  name              = "/ecs/${var.name}"
  retention_in_days = 7
}

resource "aws_ecs_cluster" "jobs" {
  name = var.name
}

resource "aws_security_group" "task" {
  name   = "${var.name}-task"
  vpc_id = data.aws_vpc.default.id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

data "aws_iam_policy_document" "assume_task" {
  statement {
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["ecs-tasks.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "execution" {
  name               = "${var.name}-execution"
  assume_role_policy = data.aws_iam_policy_document.assume_task.json
}

resource "aws_iam_role_policy" "execution" {
  role = aws_iam_role.execution.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["logs:CreateLogStream", "logs:PutLogEvents"]
      Resource = "${aws_cloudwatch_log_group.jobs.arn}:*"
    }]
  })
}

resource "aws_iam_role" "task" {
  name               = "${var.name}-task"
  assume_role_policy = data.aws_iam_policy_document.assume_task.json
}

resource "aws_iam_role_policy" "task" {
  role = aws_iam_role.task.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "s3:GetObject"
      Resource = "${aws_s3_bucket.jobs.arn}/*"
    }]
  })
}

resource "aws_ecs_task_definition" "job" {
  family                   = var.name
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.cpu
  memory                   = var.memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.task.arn

  container_definitions = jsonencode([{
    name       = "job-runner"
    image      = var.image
    entryPoint = ["sh", "-c"]
    command    = [var.command]

    logConfiguration = {
      logDriver = "awslogs"
      options = {
        awslogs-group         = aws_cloudwatch_log_group.jobs.name
        awslogs-region        = var.region
        awslogs-stream-prefix = "job-runner"
      }
    }
  }])
}

resource "aws_iam_policy" "dispatcher" {
  name = "${var.name}-dispatcher"

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid      = "PutTheJob"
        Effect   = "Allow"
        Action   = "s3:PutObject"
        Resource = "${aws_s3_bucket.jobs.arn}/*"
      },
      {
        Sid      = "RunTheTask"
        Effect   = "Allow"
        Action   = "ecs:RunTask"
        Resource = "${aws_ecs_task_definition.job.arn_without_revision}:*"
        Condition = {
          ArnEquals = { "ecs:cluster" = aws_ecs_cluster.jobs.arn }
        }
      },
      {
        Sid      = "WatchTheTask"
        Effect   = "Allow"
        Action   = "ecs:DescribeTasks"
        Resource = "arn:aws:ecs:${var.region}:${data.aws_caller_identity.this.account_id}:task/${aws_ecs_cluster.jobs.name}/*"
      },
      {
        Sid      = "PassTheTaskRoles"
        Effect   = "Allow"
        Action   = "iam:PassRole"
        Resource = [aws_iam_role.execution.arn, aws_iam_role.task.arn]
        Condition = {
          StringEquals = { "iam:PassedToService" = "ecs-tasks.amazonaws.com" }
        }
      },
      {
        Sid      = "ReadTheEvents"
        Effect   = "Allow"
        Action   = "logs:GetLogEvents"
        Resource = "${aws_cloudwatch_log_group.jobs.arn}:log-stream:*"
      }
    ]
  })
}

output "dispatcher_policy_arn" {
  value = aws_iam_policy.dispatcher.arn
}

output "run" {
  value = join(" ", [
    "cargo run -p gh-runner-fargate-ondemand --",
    "--cluster ${aws_ecs_cluster.jobs.name}",
    "--task-definition ${aws_ecs_task_definition.job.family}",
    "--bucket ${aws_s3_bucket.jobs.bucket}",
    "--log-group ${aws_cloudwatch_log_group.jobs.name}",
    "--subnets ${join(",", data.aws_subnets.default.ids)}",
    "--security-groups ${aws_security_group.task.id}",
  ])
}
