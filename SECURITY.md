# Security

## Reporting a vulnerability

Do not open a public issue for vulnerabilities that could allow arbitrary file writes, code execution, unsafe archive extraction, or release tampering.

Report them privately to the UNCHARTABLE maintainers through the security advisory feature in this GitHub repository.

Include:

- the affected version;
- a minimal reproduction archive or request;
- the expected and observed behavior;
- whether any files were written outside `CustomSongs`.

## Trust boundaries

- The desktop interface cannot provide an arbitrary archive URL to the installer command.
- Chart metadata and temporary download URLs are requested from `https://unchartable.site`.
- R2 URLs are short-lived and must use HTTPS.
- Archive contents are treated as untrusted.
- The application never executes files from a chart.
- Release signing keys must never be committed to this repository.

## Supported versions

Until version 1.0, only the newest published alpha is eligible for security fixes.
