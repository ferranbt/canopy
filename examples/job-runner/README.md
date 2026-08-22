# job-runner

Runs one GitHub Actions job on this machine, from a file describing it. The file is a
`JobMessage`, written out exactly as GitHub sent it. Nothing here talks to GitHub — whoever
took the job does that and hands it over. This is what goes in the container image the
other examples dispatch jobs to.

```sh
cargo run -p job-runner -- job.json --work ./_work   # for a person to read
cargo run -p job-runner -- job.json --json           # json events, one per line
```

Build the image from the repository root:

```sh
docker build -f examples/job-runner/Dockerfile -t job-runner .
docker run --rm -v "$PWD:/work" job-runner /work/job.json --json
```
