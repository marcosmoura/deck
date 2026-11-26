# Reviews

Reviews of deck as a whole (DECK-26, [spec](../specs/2026-11-23-reviews/00-overview.md)). Each report states the commit it reviewed, what was looked at and how, and a table of findings with what became of each: fixed (with the test that guards it), planned for a later sub-spec, a backlog task, or accepted with a reason.

| Report                                   | Reviewed                                                  | What it covers                                                                                                                                                                                                                              |
| ---------------------------------------- | --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Security](2026-11-24-security.md)       | 24 Nov 2026, `d002f75`                                    | Other local users, programs and output inside deck, hostile files and repositories, dependencies, CI and releases; `cargo-deny` (`just audit`)                                                                                              |
| [Privacy](2026-11-24-privacy.md)         | 24 Nov 2026, `a7abeac`                                    | What deck stores, logs, reads about the user and passes on, for how long, and what leaves the machine; the user page is [features/privacy.md](../features/privacy.md)                                                                       |
| [Performance](2026-11-25-performance.md) | 25 Nov 2026, `0ac69a5` (round one), `0d09d37` (round two) | Round one: keystroke latency, stalls, throughput, idle cost, memory, frame cost, profiles. Round two: start-up, live upgrade, control connections, paste, images, client memory, PNG and snapshot benchmarks; every finding with its status |

A finding's id (`A-1`, `B-2`, ...) is stable: later reports, specs and commits cite it.
