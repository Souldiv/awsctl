# awsctl

`awsctl` is a command-line tool for day-to-day work in AWS accounts reached
through IAM Identity Center (AWS SSO). It signs you in, writes an AWS profile
for the account and role you pick, and then wraps the operations you would
otherwise spell out by hand: opening a shell on an EC2 instance, forwarding a
port to an RDS database, pointing `kubectl` at an EKS cluster, and pruning old
snapshots.

You configure it against your own Identity Center portal. It ships with no
account, role or URL of its own.

## Requirements

- [AWS CLI v2](https://docs.aws.amazon.com/cli/latest/userguide/install-cliv2.html)
- The [Session Manager plugin](https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html)
  for the AWS CLI, used by the `ssh`, `ssm` and `eks --tunnel` commands
- `kubectl`, if you use the `eks` commands

## Install

With the install script, which puts `awsctl` in `~/.cargo/bin`:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Souldiv/awsctl/releases/latest/download/awsctl-installer.sh | sh
```

With Cargo:

```bash
cargo install awsctl
```

Or download the archive for your platform from the
[releases](https://github.com/Souldiv/awsctl/releases) page, then:

```bash
tar -xvf awsctl-<target>.tar.xz
sudo mv awsctl-<target>/awsctl /usr/local/bin/
```

To install somewhere else, put that directory on your `PATH` in your shell's rc
file instead.

To build from source, install a Rust toolchain and run `cargo build --release`.
The binary lands in `target/release/awsctl`.

## Configure

`awsctl` needs two things before it can sign you in: the URL of your AWS access
portal and the region your Identity Center instance runs in. Find the URL in the
IAM Identity Center console, under Settings; it looks like
`https://your-sso-portal.awsapps.com/start`.

The first time you run an `sso` command, `awsctl` prompts you for both and
remembers them:

```bash
awsctl sso --get-access-token
```

To set them without the prompt, copy `Config.toml` from this repository, edit
the values, and load it:

```bash
awsctl config --update ./Config.toml
```

The same file lists the roles and regions `awsctl` offers when you choose one,
and the short profile names described under [Naming your
profiles](#naming-your-profiles). `awsctl config --show` prints the current
config, `awsctl config --location` prints its path, and `awsctl config edit`
opens it in `$EDITOR`.

## Use

Sign in and choose the accounts and role you want:

```bash
awsctl sso --get-access-token   # authenticate in the browser
awsctl sso --setup-profile      # pick accounts and a role, write the AWS profiles
awsctl sso --refresh            # renew the session and the [default] credentials
awsctl sso --keep-alive         # refresh the session in the background
```

Pass `--account` and `--role` to skip the interactive pick, and `--force` to
discard a cached token. `--get-access-token` also hands the new token to the
AWS CLI's cache, so every profile `awsctl` wrote works again without re-running
`--setup-profile`.

`--setup-profile` lets you tick more than one account. Each one gets a profile
in `~/.aws/config`, so you reach the others without signing in again:

```bash
aws s3 ls --profile au
AWS_PROFILE=au aws eks list-clusters
```

Tick `All accounts` at the top of the list to write a profile for every account
your sign-in can reach, in one pass. Each account gets the role you pick when it
grants that role, and otherwise the first role it does grant; an account that
grants none is skipped. Naming follows the rules under [Naming your
profiles](#naming-your-profiles), as for a single pick.

After that one-time setup, `awsctl sso --refresh` is the only command you need
day to day. It asks no setup questions. It renews the SSO token, which every
written profile mints its credentials from, and re-mints the `[default]`
credentials in `~/.aws/credentials` for the profile `[default]` holds. While the
cached token is still valid it reuses it, so there is no browser round trip;
once the token has expired, it opens the browser to sign you in again. Add
`--force` to sign in again even when the token is still valid.

Those profiles share one `[sso-session awsctl]` block, which the AWS CLI and the
SDKs use to mint and refresh role credentials on their own. After the session
itself expires, `aws sso login --sso-session awsctl` renews every profile at
once.

The account you pick for `[default]` also has its credentials written to
`~/.aws/credentials`, as before, so commands you run without a profile still act
on that account. The `awsctl` subcommands below read those credentials rather
than a profile, so they follow the account you picked for `[default]`.

To move `[default]` to another of those profiles later, run
`awsctl sso --switch-default au`, or leave the name off to pick from a list. It
reuses the cached access token, so there is no browser round trip, and it
rewrites only `[default]` in `~/.aws/credentials`. `awsctl sso --list-profiles`
prints the profiles `awsctl` has written, with a `*` on the one `[default]`
currently holds.

### Naming your profiles

A profile name is what you type, so `awsctl` keeps it short. It lists every
account it finds under `[profile_names]` in your config:

```toml
[profile_names]
"labflow-dev-customer" = "dev"
"labflow-saas-au" = "au"
```

Edit the right-hand side to whatever you want to type. The left-hand side takes
an account id as well, which outlives a rename of the account.

An account you have not named gets its account name lowercased, with anything
that would need quoting replaced by `-`. When two accounts want one name, the
first keeps it and the second gets its account id appended; `awsctl` prints
which account that was.

The remaining commands act on the profile you selected:

| Command | What it does |
| --- | --- |
| `awsctl info --accounts` | List the accounts you can reach |
| `awsctl info --roles` | List the roles available in the current account |
| `awsctl ssh ec2` | Open a Session Manager shell on an EC2 instance |
| `awsctl ssh rds --port 3333` | Forward a local port to an RDS instance |
| `awsctl ssm --create-document --document-path <file>` | Register an SSM document |
| `awsctl eks --list` | List EKS clusters |
| `awsctl eks --update` | Write the cluster's context into your kubeconfig |
| `awsctl eks --tunnel` | Reach private clusters' APIs through their bastions over SSM |
| `awsctl eks --tunnel-status` | Show the tunnels running `awsctl` processes keep open |
| `awsctl snapshot <aws\|eks\|cnpg> -n <namespace> -b <date>` | Count or delete snapshots older than a date |

`awsctl snapshot` lists what it matched, or counts it with `--count`. It deletes
only when you pass `--delete-all`, and asks first unless you also pass `--yes`.

Run `awsctl --help`, or `awsctl <command> --help`, for the full set of flags.

### Reaching a private EKS cluster over SSM

`awsctl eks --tunnel` reaches a cluster whose API endpoint is private without a
VPN. It forwards a local port through a bastion in the cluster's VPC to the
endpoint on 443, using the `AWS-StartPortForwardingSessionToRemoteHost` Session
Manager document, and writes a kubeconfig that points at that port:

```bash
awsctl eks --tunnel --profile <profile> --cluster <cluster>
```

Leave it running; Ctrl-C closes the forward and ends the session. In another
terminal:

```bash
KUBECONFIG=~/.kube/ssm-<cluster>.yaml kubectl get nodes
```

Each cluster gets its own local port, derived from its name. The kubeconfig's
`server` is `https://127.0.0.1:<port>`, and its `tls-server-name` is the real
endpoint, so the certificate is still checked against the cluster's name and
CA. Its user runs `aws eks get-token` with the same profile and region.

- `--profile` picks the AWS profile. Without it, `awsctl` uses its own
  credentials for the lookups and writes the profile it set up for that
  account into the kubeconfig, or `default` when it wrote none.
- `--region` overrides the profile's region.
- `--local-port <port>` overrides the derived port. `awsctl` stops if the port
  is already in use.
- `--bastion <instance-id>` skips discovery. Otherwise `awsctl` looks in the
  cluster's VPC for a running instance tagged `Component=bastion`, then one
  named `*-jump-host` (preferring `<label>-jump-host` with `--account-label
  <label>`), then one named `bastion-host`. It asks when several match.
- `--kubeconfig <file>` writes the tunnel's cluster, user and context into
  another file, such as `~/.kube/config`. Other entries are kept, the file's
  current context is left alone unless it has none, and the file is rewritten
  readable only by you.

#### Several clusters at once

Name `--cluster` once per cluster. Write `<profile>:<cluster>` to reach each
one through its own account:

```bash
awsctl eks --tunnel --cluster dev:<cluster> --cluster prod:<other-cluster>
```

One `awsctl` keeps every tunnel open. Each still gets its own port and its own
`~/.kube/ssm-<cluster>.yaml`, and every context is also merged into
`~/.kube/awsctl-tunnels.yaml` (or the file `--kubeconfig` names), so one
kubeconfig reaches them all:

```bash
export KUBECONFIG=~/.kube/awsctl-tunnels.yaml
kubectl --context ssm-<cluster> get nodes
kubectl --context ssm-<other-cluster> get nodes
```

`--profile`, `--region` and `--account-label` apply to every cluster that does
not set its own. `--local-port` and `--bastion` name one tunnel, so they work
only with a single cluster. When two names derive the same port, the one that
sorts later takes the next free port. A cluster name may appear only once,
because its context and kubeconfig are named after it.

To open the same set every day, list it under `[tunnel_groups]` in awsctl's
config (`awsctl config edit`), and open it by name:

```toml
[tunnel_groups]
work = [
    "dev:<cluster>",
    { cluster = "<other-cluster>", profile = "prod", local_port = 21443, bastion = "<instance-id>" },
]
```

```bash
awsctl eks --tunnel --group work
```

A table entry can set `cluster`, `profile`, `region`, `local_port`, `bastion`
and `account_label`. `--cluster` adds clusters to a group's.

#### Staying connected

Session Manager ends a session after 20 idle minutes by default, when it is
terminated, or when the laptop sleeps or changes network. `awsctl` reopens a
tunnel whenever its session ends, waiting 1 second, then 2, 4 and so on up to
5 minutes between attempts. A tunnel that stayed up for a minute starts the
wait over.

While a tunnel is up, `awsctl` asks the cluster for `/livez` through it every
2 minutes, with no credentials, over TLS checked against the cluster's CA. The
traffic keeps the session from idling out, and two failed checks in a row
reopen a tunnel that is stuck.

When the credentials behind a tunnel have expired or are missing, `awsctl`
says so, marks the tunnel `needs login`, and stops reopening it. It checks
again every minute with a read-only call, and reconnects once you have run
`aws sso login --profile <profile>` or `awsctl sso --refresh`. Other tunnels
keep running meanwhile.

Ctrl-C, `SIGTERM` or closing the terminal stops every tunnel and terminates
every session. A second Ctrl-C exits without waiting for them.

#### Seeing what is running

From any terminal:

```bash
awsctl eks --tunnel-status
```

```text
CLUSTER          PROFILE  PORT   BASTION              STATE                UPTIME  LAST DROP
<cluster>        dev      2xxxx  <instance-id>        up                   1h02m   25m10s ago: Your session timed out due to inactivity…
<other-cluster>  prod     21443  <instance-id>        needs login (4m10s)  -       4m10s ago: credentials expired or missing; run `aws sso…

awsctl pid 12345 has supervised 2 tunnel(s) for 3h15m
```

Each running `awsctl eks --tunnel` keeps its state in
`tunnels/<pid>.json` next to awsctl's config file (`awsctl config --location`),
readable only by you, and removes it on exit. A file left by an `awsctl` that
crashed is spotted by its process id, left out, and removed.

The bastion must be managed by Systems Manager, and your role needs
`eks:DescribeCluster`, `ec2:DescribeInstances`,
`ssm:DescribeInstanceInformation`, `ssm:StartSession` on the bastion and the
port-forwarding document, and `ssm:TerminateSession`. Unless your account sets
`ssm:SessionDocumentAccessCheck`, `ssm:StartSession` on an instance also allows
a shell on it, and the port-forwarding document forwards to any host the
bastion can reach. `awsctl` itself only ever forwards to the cluster endpoint
on 443, but the grant is wider than that.

### Known issue

The `ssm` subcommand also declares `--list-inventory` and `--describe-document`,
but neither can be reached: `--document-path` is required on every `ssm`
invocation, and its short form collides with `--describe-document`'s. Only
document creation works today.

## Contributing

See [DEVELOPER.md](./DEVELOPER.md) for the build and release process.

## License

This project is licensed under the MIT License. See [LICENSE](./LICENSE).
