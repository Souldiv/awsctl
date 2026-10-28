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
  for the AWS CLI, used by the `ssh` and `ssm` commands
- `kubectl`, if you use the `eks` commands

## Install

Download the archive for your platform from the
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
awsctl sso --keep-alive         # refresh the session in the background
```

Pass `--account` and `--role` to skip the interactive pick, and `--force` to
discard a cached token.

`--setup-profile` lets you tick more than one account. Each one gets a profile
in `~/.aws/config`, so you reach the others without signing in again:

```bash
aws s3 ls --profile au
AWS_PROFILE=au aws eks list-clusters
```

Those profiles share one `[sso-session awsctl]` block, which the AWS CLI and the
SDKs use to mint and refresh role credentials on their own. After the session
itself expires, `aws sso login --sso-session awsctl` renews every profile at
once.

The account you pick for `[default]` also has its credentials written to
`~/.aws/credentials`, as before, so commands you run without a profile still act
on that account. The `awsctl` subcommands below read those credentials rather
than a profile, so they follow the account you picked for `[default]`.

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
| `awsctl snapshot <aws\|eks\|cnpg> -n <namespace> -b <date>` | Count or delete snapshots older than a date |

`awsctl snapshot` lists what it matched, or counts it with `--count`. It deletes
only when you pass `--delete-all`, and asks first unless you also pass `--yes`.

Run `awsctl --help`, or `awsctl <command> --help`, for the full set of flags.

### Known issue

The `ssm` subcommand also declares `--list-inventory` and `--describe-document`,
but neither can be reached: `--document-path` is required on every `ssm`
invocation, and its short form collides with `--describe-document`'s. Only
document creation works today.

## Contributing

See [DEVELOPER.md](./DEVELOPER.md) for the build and release process.

## License

This project is licensed under the MIT License. See [LICENSE](./LICENSE).
