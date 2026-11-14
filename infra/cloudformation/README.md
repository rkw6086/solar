# Solar EC2 host foundation

`solar-host.yaml` prepares one native Amazon Linux 2023 x86_64 machine. It does not install a Solar binary, run Orbits, configure a relay, or open a public application port. The remote host and guided installer described in [the remote design](../../docs/remote-orbits.md) must exist before application deployment. This template has not been deployed by this change.

The template uses an existing public subnet/VPC and key pair. Supply the same VPC that owns the subnet. The subnet must have an Internet Gateway route; the instance receives a public IPv4 address. No new VPC, NAT gateway, Elastic IP, or DNS record is created. Only TCP 22 from the supplied administrator CIDR is allowed inbound. Use your current public IPv4 address with `/32`; the template accepts only `/24` through `/32`.

Required inputs:

| Parameter | Value |
| --- | --- |
| `VpcId` | Existing `vpc-…` containing the subnet |
| `SubnetId` | Existing public `subnet-…` in that VPC |
| `KeyPairName` | Existing EC2 key-pair name; retain its private key locally |
| `AdminCidr` | Administrator source address, e.g. `203.0.113.10/32` replaced by your real IP |

Optional inputs are `InstanceType` (`t3.small` default, x86_64 only), `RootVolumeGiB` (`32` default), `LatestAmiId` (the AWS AL2023 public SSM parameter), and `EnableSessionManager` (`false` default). Session Manager enables the documented AWS instance-core policy, not arbitrary account administrator permissions. The host uses IMDSv2 and an encrypted gp3 root volume. It creates a separate `solar` service account under `/home/solar`; SSH administration uses `ec2-user`.

Use a change set to review the actual resources before creating them. The commands below are deployment instructions for later use; they have not been run as part of preparing this template. Replace every example identifier and select the intended AWS profile/region first.

```sh
aws cloudformation create-change-set \
  --stack-name solar-host \
  --change-set-name solar-host-initial \
  --change-set-type CREATE \
  --template-body file://infra/cloudformation/solar-host.yaml \
  --capabilities CAPABILITY_IAM \
  --parameters \
    ParameterKey=VpcId,ParameterValue=vpc-REPLACE \
    ParameterKey=SubnetId,ParameterValue=subnet-REPLACE \
    ParameterKey=KeyPairName,ParameterValue=REPLACE \
    ParameterKey=AdminCidr,ParameterValue=203.0.113.10/32

aws cloudformation wait change-set-create-complete \
  --stack-name solar-host --change-set-name solar-host-initial
aws cloudformation describe-change-set \
  --stack-name solar-host --change-set-name solar-host-initial

# After reviewing the change set and the expected costs:
aws cloudformation execute-change-set \
  --stack-name solar-host --change-set-name solar-host-initial
aws cloudformation wait stack-create-complete --stack-name solar-host
aws cloudformation describe-stacks --stack-name solar-host \
  --query 'Stacks[0].Outputs' --output table
```

Confirm the returned instance passes EC2 status checks, then use its SSH output with the matching local private key or an SSH-config alias. Verify the new host's fingerprint before accepting it. Run `cloud-init status --wait` and check `/home/solar/.solar/provisioning-status.txt` through the authenticated connection. The proposed guided installer can then transfer a real signed/checksummed `solar-host` release, install its systemd unit, start it, and perform pairing. No provider credentials belong in CloudFormation parameters, outputs, tags, or EC2 user data.

Creating the stack incurs EC2, EBS, and public IPv4 charges. This first template retains the EC2 instance, security group, and optional instance role/profile on stack deletion or replacement so a stack operation does not silently erase Orbit data. Retained EC2 continues to run and cost money until explicitly stopped or terminated. Before cleanup, back up `/home/solar/.solar`, then terminate the retained instance and remove its security group and any optional IAM profile/role. Manual instance termination deletes this template's root volume. The future wizard must expose this retention and cleanup state instead of implying that deleting a stack removed everything.

For local validation without contacting AWS:

```sh
cfn-lint infra/cloudformation/solar-host.yaml
```

Validation recorded for this change: `cfn-lint` 1.60.1 passed with no findings; local YAML/security assertions and `git diff --check` passed. This is static validation, not a CloudFormation deployment or an EC2 boot test.
