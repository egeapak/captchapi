# Security Policy

## Reporting a Vulnerability

We take security seriously. If you discover a security vulnerability in CaptchAPI, please report it responsibly.

### How to Report

Please use [GitHub's private vulnerability reporting](https://github.com/egeapak/captchapi/security/advisories/new) to report security issues. This ensures the vulnerability is handled privately until a fix is available.

**Do NOT open a public issue for security vulnerabilities.**

### What to Include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

### Response Timeline

- **Acknowledgment**: Within 48 hours
- **Initial Assessment**: Within 1 week
- **Fix Timeline**: Depends on severity, typically within 2 weeks for critical issues

### Scope

The following are in scope:
- Authentication and authorization bypasses
- SQL injection or other injection attacks
- Information disclosure
- Denial of service vulnerabilities
- Cryptographic weaknesses

### Known Limitations

- **API key hashing**: CaptchAPI currently uses SHA256 + salt for API key hashing. While adequate for API key verification, migration to Argon2 is planned for a future release. This is tracked as a known improvement area.

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.0.x   | :white_check_mark: |
| < 1.0   | :x:                |
