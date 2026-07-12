# cli — delta for readable-diff-output

## ADDED Requirements

### Requirement: diff flags
`oops diff` SHALL accept a `--porcelain` flag selecting the stable
machine-readable output format (see the diff capability). It appears in
`oops diff --help`. No other verb gains flags in this change.

#### Scenario: Flag is documented
- **WHEN** `oops diff --help` is invoked
- **THEN** `--porcelain` is listed with a description marking it as the stable script/agent interface
