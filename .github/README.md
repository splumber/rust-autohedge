# GitHub Actions for rust-autohedge

This repository includes comprehensive GitHub Actions workflows for CI/CD, security, and quality assurance.

## Workflows

### 1. CI Workflow (`ci.yml`)
**Trigger**: Push and Pull Requests to master/main branch

**Jobs**:
- **Build**: Compiles the project on stable, beta, and nightly Rust versions
- **Test**: Runs all unit and integration tests in both debug and release modes
- **Clippy**: Runs Rust's linter to catch common mistakes and improve code quality
- **Format**: Checks code formatting with rustfmt
- **Security Audit**: Scans dependencies for known security vulnerabilities using cargo-audit
- **Coverage**: Generates code coverage reports and uploads to Codecov

**Purpose**: Ensures code quality, correctness, and security on every change.

### 2. CodeQL Security Scan (`codeql.yml`)
**Trigger**: Push, Pull Requests, and weekly schedule (Mondays at 00:00 UTC)

**Jobs**:
- Performs static code analysis to detect security vulnerabilities
- Runs GitHub's CodeQL analysis engine
- Results appear in the Security tab

**Purpose**: Continuous security monitoring and vulnerability detection.

### 3. Release Workflow (`release.yml`)
**Trigger**: Git tags matching `v*.*.*` pattern (e.g., v1.0.0)

**Jobs**:
- Creates GitHub releases
- Builds binaries for Linux, macOS, and Windows
- Uploads release artifacts automatically

**Usage**: Create a git tag to trigger a release:
```bash
git tag v1.0.0
git push origin v1.0.0
```

**Purpose**: Automated multi-platform releases.

### 4. Dependabot (`dependabot.yml`)
**Trigger**: Scheduled (Weekly on Mondays at 09:00 UTC)

**Updates**:
- Rust dependencies (Cargo)
- GitHub Actions versions

**Purpose**: Keeps dependencies up-to-date automatically, reducing security risks and ensuring compatibility.

### 5. Benchmark Workflow (`benchmark.yml`)
**Trigger**: Push and Pull Requests to master/main branch, manual dispatch

**Jobs**:
- Runs performance benchmarks (if defined)
- Tracks performance changes over time
- Alerts on significant performance regressions (>200%)

**Purpose**: Performance monitoring and regression detection.

### 6. Documentation Workflow (`docs.yml`)
**Trigger**: Push and Pull Requests to master/main branch, manual dispatch

**Jobs**:
- Builds Rust documentation with `cargo doc`
- Checks for broken links
- Deploys documentation to GitHub Pages (on master/main pushes)

**Purpose**: Automated documentation generation and hosting.

### 7. Docker Workflow (`docker.yml`)
**Trigger**: Push to master/main, tags, Pull Requests, manual dispatch

**Jobs**:
- Builds Docker image
- Pushes to GitHub Container Registry (ghcr.io)
- Tags with branch names, PR numbers, versions, and commit SHAs

**Purpose**: Containerized deployment support.

### 8. Dependency Review (`dependency-review.yml`)
**Trigger**: Pull Requests

**Jobs**:
- Reviews dependency changes in PRs
- Identifies security vulnerabilities in new dependencies
- Fails if dependencies with moderate or higher severity issues are added

**Purpose**: Proactive security review for dependency changes.

## Additional Suggestions

### Suggested Future Actions

1. **Continuous Deployment**
   - Automatically deploy to staging/production on master branch updates
   - Use GitHub Environments for deployment approval gates

2. **Performance Profiling**
   - Add flamegraph generation for performance analysis
   - Use tools like `cargo flamegraph` or `perf`

3. **End-to-End Testing**
   - Add E2E tests that simulate real trading scenarios
   - Mock Alpaca API responses for deterministic testing

4. **Nightly Builds**
   - Daily builds to catch issues early
   - Test against latest dependencies

5. **License Compliance**
   - Use `cargo-license` to check dependency licenses
   - Ensure compliance with your license requirements

6. **SBOM Generation**
   - Generate Software Bill of Materials (SBOM)
   - Use `cargo-sbom` for tracking dependencies

7. **Mutation Testing**
   - Add mutation testing with `cargo-mutants`
   - Improve test quality by finding weak spots

8. **Fuzz Testing**
   - Add fuzzing with `cargo-fuzz`
   - Test edge cases automatically

9. **Stale PR/Issue Management**
   - Automatically label and close stale issues/PRs
   - Use GitHub's stale action

10. **Automated Changelog**
    - Generate changelogs automatically from commits
    - Use conventional commits and `git-cliff` or similar

11. **Matrix Testing**
    - Test on multiple operating systems (Linux, macOS, Windows)
    - Test with different feature flags

12. **Pre-commit Hooks**
    - Add `.pre-commit-config.yaml` for local validation
    - Enforce formatting and linting before commits

## Cache Strategy

All workflows use GitHub Actions cache to speed up builds:
- **Cargo registry**: Caches downloaded crate metadata
- **Cargo git**: Caches git dependencies
- **Target directory**: Caches compiled artifacts

This significantly reduces build times on subsequent runs.

## Security Considerations

- **Secrets**: Never commit API keys or secrets
- **Dependabot**: Reviews and updates dependencies automatically
- **CodeQL**: Scans for vulnerabilities weekly
- **Dependency Review**: Blocks PRs with vulnerable dependencies
- **Security Audit**: Runs on every CI build

## Monitoring and Badges

Add these badges to your README.md:

```markdown
![CI](https://github.com/splumber/rust-autohedge/workflows/CI/badge.svg)
![Security](https://github.com/splumber/rust-autohedge/workflows/CodeQL%20Security%20Scan/badge.svg)
[![codecov](https://codecov.io/gh/splumber/rust-autohedge/branch/master/graph/badge.svg)](https://codecov.io/gh/splumber/rust-autohedge)
```

## Maintenance

- Review Dependabot PRs regularly
- Monitor CodeQL security alerts
- Check workflow run status in the Actions tab
- Update actions versions when Dependabot suggests

## Cost Optimization

- Caching reduces build times and costs
- Matrix builds run in parallel
- Conditional jobs (e.g., release only on tags)
- Dependency review only on PRs

## Troubleshooting

### Build Failures
- Check the Actions tab for detailed logs
- Ensure all tests pass locally with `cargo test`
- Verify formatting with `cargo fmt --check`
- Run clippy locally with `cargo clippy`

### Docker Build Failures
- Test Docker build locally: `docker build -t rust-autohedge .`
- Check Dockerfile for correct paths
- Ensure all dependencies are included

### Release Issues
- Verify tag format matches `v*.*.*`
- Check release workflow permissions
- Ensure GITHUB_TOKEN has sufficient permissions

## Contributing

When contributing, ensure:
1. All CI checks pass
2. Code is formatted with `cargo fmt`
3. No clippy warnings
4. Tests are added for new features
5. Documentation is updated

## Support

For issues with workflows:
1. Check the Actions tab for error messages
2. Review workflow logs
3. Open an issue with the error details
