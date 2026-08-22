# gh-runner-registration

Registers a self-hosted runner with GitHub and writes the credentials the other examples
authenticate with. Run it once per runner.

Take a registration token from **Settings → Actions → Runners → New self-hosted runner**
on the repository, or:

```sh
gh api -X POST repos/OWNER/REPO/actions/runners/registration-token --jq .token
```

```sh
cargo run -p gh-runner-registration -- \
  --url https://github.com/OWNER/REPO \
  --token AAAA... \
  --name canopy \
  --labels gpu,fast \
  --credentials credentials.json
```

The token is single-use and short-lived; `credentials.json` is not. It holds the runner's
private key and lasts until the runner is deleted.
