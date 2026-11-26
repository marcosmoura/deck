# Security

## Reporting a vulnerability

Report it privately, not in a public issue: open a report with GitHub's private vulnerability reporting, under **Security** → **Report a vulnerability** on [the repository](https://github.com/marcosmoura/deck/security/advisories/new). Only the maintainer sees it.

Say what you did, what happened and what you expected, with the deck version (`deck --version`) and the operating system. A way to reproduce it helps most.

## What counts

deck runs programs for one user on one machine. The daemon's socket, state and log are private to that user, and a program that can reach the socket can do what the user can do in deck. The boundaries deck defends, and the reviews made of them, are in [docs/reviews/](docs/reviews/).
