# CI runners: options and costs (PDT-27, 2026-10-18)

Research for PDT-27. Prices are from vendor pages fetched on 2026-10-18; ⚠ marks figures that are vendor claims,
third-party, or could not be confirmed. Current setup: docs/superpowers/notes/ci.md.

## Summary

- Half of the Windows job's time does nothing Windows-specific: the browser-UI tests (vitest) take **6m37s** on
  `windows-latest` and **33s** on the Mac mini. Running them only on macOS takes the Windows checks from ~16 to ~9 min,
  for free.
- `windows-latest` is small for this repo: an internal repo gets the private-repo size, **2 vCPU / 8 GB**.
- The Mac mini is a base **M4, 10 cores, 16 GB**, and it also runs the agents and the stable PDT. RAM is the likely
  pinch, and it is too small to wall CI off in a VM (a 4-core / 8 GB VM leaves 8 GB for everything else).
- The repo is **internal**, so the org is on **GitHub Enterprise Cloud**. That gives it 50,000 included Actions
  minutes a month, larger runners and **merge queue**.
- **Merge queue fixes PDT-24 outright.** GitHub tests "main + the PR" itself and merges, so no PR branch ever needs
  updating, no token workaround is needed, and `update-prs.yml` and the GitHub App from PR #35 go away. Buildkite
  supports GitHub's merge queue natively, so this carries over when Buildkite arrives.
- Cheapest way to take CI off the Mac mini while staying on Actions: a third-party runner with a disk cache that
  persists between runs (Namespace is the best fit). Estimate **~$115/month** for macOS and Windows checks; CircleCI
  is ≈$200 for the same work, since its macOS minutes cost twice as much. Most of any hosted bill is macOS minutes.
- Buildkite is free with your own agents, but that means running agents on hardware you own (the same Mac mini
  problem), and it has no hosted Windows agents yet. Make the pipeline portable now (one command per check, which
  any CI can call); add Buildkite when the work laptop needs it.

## Where the time goes today

Run 39160248113 (push to main, all jobs):

| Job | Runner | Time | Main costs |
|---|---|---|---|
| build (macos-latest) | Mac mini (self-hosted, warm) | 2.4 min | nextest 31 s, vitest 33 s |
| build (windows-latest) | GitHub 2 vCPU | 15.8 min | **vitest 6m37s**, nextest build 2 min + run 3m17s, clippy 40 s, setup ~1 min |
| bundle (windows-latest, nsis) | GitHub 2 vCPU | 9.8 min | release build |
| bundle (macos-latest, app) | GitHub M1 3 vCPU | 3.1 min | release build, sign, notarize |

On Windows, vitest's time is spread across tests, jsdom setup and imports (46/25/21%). That points to slow process and
file-system work on a 2-vCPU Windows VM, not a single bad test. The web code is the same on every OS. The slowest
Rust tests on Windows are 5–9 s each (pdt-host supervise, activity, migrations).

## The PDT-24 problem and how each option handles it

The "Main" ruleset requires PRs to be up to date. When main moves, `update-prs.yml` updates each auto-merge PR with
`GITHUB_TOKEN`. A `pull_request` run caused by `GITHUB_TOKEN` is created as `action_required` and waits for someone
to approve it (behaviour since GitHub's 2026-06-11 change), so the dispatch workaround was added. PR #35 swaps in a
GitHub App token, which is one more secret plus an App to maintain.

| Approach | Fixes PDT-24? | Notes |
|---|---|---|
| **GitHub merge queue** (Enterprise Cloud: available ⚠ docs say "private repos on Enterprise Cloud" and don't mention internal, so try it in the ruleset) | Yes, and the update step is gone | Add `on: merge_group`, and have `changes` run every job for a merge group. Queue runs come from GitHub itself, not a token, so they always start. Works with self-hosted and third-party runners (it is just `runs-on`) and with Buildkite. A PR costs two CI runs (PR + queue), but the queue run can replace the main push run's checks. |
| GitHub App token (PR #35) | Yes | Keeps the update-branch loop; one App plus a private key secret. Actions only. |
| Drop "require up to date" | Sidesteps it | Simplest; a semantic conflict between two PRs shows up only on main. |
| Buildkite / CircleCI | Probably ⚠ | They start builds from webhooks, and the `GITHUB_TOKEN` rule only stops *Actions* runs; not documented by either vendor, so check webhook deliveries once. |
| Faster runner providers (Namespace, Blacksmith, …) | No | Still GitHub Actions: the same token rule applies. |

## Options and costs

Volume: CI ran 159 times in its first 3.5 days (2026-10-15 to 10-18), so ~400 runs/month is a fair planning number.
Monthly costs below cover the **checks only**, with bundles left on GitHub's runners (within the included minutes):

- macOS checks: every code PR and main run, ~4 min each with a warm cache (the Mac mini takes 2.4), so ≈1,600 min.
- Windows checks: main and labelled PRs only, ~7 min after A, so ≈1,000 min.

Per-run times are estimates, not measurements. macOS minutes cost 5–20× Linux or Windows minutes, so they dominate
every hosted total.

### A. Free changes (do these whatever else is chosen)

- Run vitest, web typecheck and the Rust fmt check on macOS only (keep `pnpm --dir web build` on Windows if
  `cargo test -p pdt-desktop` needs it). Saves ~6.5 min per Windows run.
- Build test binaries once (`cargo nextest archive`) if the Windows job is ever split.
- Optional: a Dev Drive (`samypr100/setup-dev-drive`) is claimed to give ~25% ⚠. uv found the `D:` drive on standard
  runners already as fast and only uses a Dev Drive on larger runners. Low priority.
- Opt-in `rust-lld` on MSVC: no measurements found ⚠.

### B. Stay on GitHub Actions, move off the Mac mini

| Option | macOS checks | Windows checks | Est. $/month | Cache | Notes |
|---|---|---|---|---|---|
| **GitHub standard** (`macos-latest` M1 3 vCPU / `windows-latest` 2 vCPU) | ~6–8 min ⚠ | ~9 min after A | **$0** if within the 50k included minutes (slower runners, so more minutes: ≈2,800 macOS + ≈1,000 Windows; ≈30k if the old 10×/2× weighting still applies ⚠), else ≈$185 | rust-cache restores ~1 GB per run | Slowest; no spend to approve. |
| GitHub larger runners | xlarge M2 Pro 5 CPU, $0.102/min | 8-core 32 GB, $0.042/min | ≈$165 mac + ≈$40 win ≈ **$205** | rust-cache only (no persistent disk) | Never covered by included minutes; needs a card. |
| **Namespace** | M4 Pro 6 vCPU, $0.06/min (prepaid) | 8 vCPU, $0.016–0.024/min | ≈$95 mac + ≈$20 win ≈ **$115** + cache storage | **Cache volumes**: local NVMe carried between runs, `rust` mode. ⚠ Windows cache volumes were early access in July 2026 | Fastest in RunsOn's Rust benchmark (Linux, 4 vCPU: 1m47s vs GitHub 3m53s). Vendor case study: a warm Windows Rust job went from 9m31s to 44s ⚠. Developer plan $0 + pay as you go, 30-day trial. |
| Blacksmith | M4 6 vCPU, $0.08/min | Server 2025 (**beta**), ~$0.032/min at 8 vCPU ⚠ | ≈$130 mac + ≈$30 win ≈ **$160**, less free minutes | Sticky disks are Ubuntu only | 3,000 free 2-vCPU min/month. |
| Depot | M4 8 vCPU, $0.08/min, **only on the $200/month plan** | 4/8 vCPU $0.016/$0.032 | ≥$200 | Remote cache, no RAM disk on Windows | Too expensive for macOS here. |
| WarpBuild | M4 Pro 6 vCPU, $0.08/min | 4 vCPU $0.016 | ≈$130 mac + ≈$15 win ≈ **$145** | **No caching on Windows** | — |
| RunsOn (your AWS) | — | Spot, ~$0.002/min ⚠ | ≈$5 + €300/yr licence | S3 | No macOS; Windows boot takes 1–3 min. |

Gone: BuildJet (shut down 2026-03-31), Cirrus CI and Cirrus Runners (shut down 2026-06-01; the team joined OpenAI;
Tart stays open source).

GitHub's planned $0.002/min fee for self-hosted runners (announced Dec 2025 for March 2026) was postponed with no
new date. Self-hosted runners are still free.

### C. Your own hardware (any CI system)

| Option | Cost | Notes |
|---|---|---|
| Keep the Mac mini, limit CI harder | $0 | `CARGO_BUILD_JOBS=4`, `taskpolicy -b` (efficiency cores only: much slower), smaller target cap. Still shares 16 GB. |
| Tart VM on the Mac mini | $0 (free under 100 cores) | Hard CPU/RAM caps and throwaway disks, but 16 GB is too small to split. Apple allows 2 macOS VMs per host. |
| Second Mac mini as a CI box | **$899** (M6 16 GB/256 GB; ~$780 street); 512 GB advised | Removes contention for good; warm builds like today. |
| Windows mini PC (Ryzen 7, 32 GB) | ~$400–770 ⚠ (2026 RAM prices) | ~4–6× the CPU of `windows-latest` ⚠; warm `target/`. Another machine to keep patched. |
| Scaleway Mac mini M4 (cloud) | €149/month, 24 h minimum | Same as a second Mac mini, rented. |
| AWS EC2 Mac (`mac-m4.metal`) | ~$900/month on demand ⚠ | Too expensive. |

### D. Other CI systems

| Platform | macOS | Windows | Est. $/month here | Notes |
|---|---|---|---|---|
| **Buildkite, Free plan + own agents** | own agent | own agent | **$0** + hardware (C) | 5 users, 10 concurrent jobs, 30-day history. GitHub Checks via its App; supports merge queue. Windows agent runs cmd/PowerShell (no Git Bash). |
| Buildkite Pro + hosted macOS | M4 6 vCPU $0.12/min (12 vCPU $0.24) | **Hosted Windows is private preview, no price** | ≈$190 mac + $30/user ≈ **$220** (Windows elsewhere) | macOS cache volumes (20 GB minimum, 14 days). Windows would still need GitHub or your own agent. |
| CircleCI | M4 Pro Medium 200 credits/min = $0.12/min | Medium (4 CPU) 40 credits/min = $0.024/min | ≈$190 mac + ≈$25 win − $18 free credits ≈ **$200** | Credits are $15 per 25,000. Free plan: 30,000 credits/month (= 150 macOS or 750 Windows min), 1 macOS job at a time. Performance plan: 5 users included, then $15/user. Windows Medium is 4 CPU, twice GitHub's standard runner. |
| GitLab CI (mirror) | beta, Premium only | beta | ≥$29/user + minutes | Second git host; poor fit. |
| Codemagic / Bitrise / Semaphore | $0.09–0.114/min | Codemagic only ($0.045/min) | ≈$150–230 | Built for mobile apps, or metered even on your own agents. |
| Dagger | — | — | — | Linux containers only: no help for macOS signing or NSIS. |

## Keeping the pipeline portable (for Buildkite later)

Put each check behind one command that any CI calls, so a CI file only picks the machine, installs tools, exposes
secrets as env vars and runs that command:

- `cargo xtask ci <check|test|web|bundle-macos|bundle-windows>` (Rust is on every agent, and no bash/PowerShell
  split on Windows), or `just`/`mise` tasks.
- `scripts/sign-macos.sh` / `notarize-macos.sh` already read env vars; keep it that way.
- The `changes` job's docs-only and label logic becomes a script both systems use. Buildkite has `if_changed` and
  dynamic pipelines (`buildkite-agent pipeline upload`).
- Keep the required check names fixed. Buildkite can post a GitHub Check with any name (`notify: github_check`), so
  the ruleset can move over without renaming. Change the ruleset's "expected source" app when it does.

## Recommendation

1. **Now, free:** take vitest (and the other OS-independent web checks) off the Windows job.
2. **PDT-24:** switch "Main" to merge queue instead of "require up to date"; add `merge_group` to ci.yml; remove
   `update-prs.yml`; close PR #35. Portable to Buildkite.
3. **Mac mini relief:** move `build (macos-latest)` (and the Windows checks) to Namespace with cache volumes on its
   free trial, and measure warm times. Fallbacks:
   - GitHub's standard runners, if paying nothing matters more than speed.
   - A second Mac mini ($899) plus a Windows mini PC, if you'd rather own hardware than pay monthly. That route
     also gives Buildkite its free own-agent setup.
4. **Alongside step 3:** move the check steps behind `cargo xtask ci …` so a Buildkite pipeline can call the same
   commands.

## Sources

GitHub: [runner pricing](https://docs.github.com/en/billing/reference/actions-runner-pricing) ·
[hosted runner specs](https://docs.github.com/en/enterprise-cloud@latest/actions/reference/runners/github-hosted-runners) ·
[larger runners](https://docs.github.com/en/actions/reference/runners/larger-runners) ·
[merge queue](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue) ·
[GITHUB_TOKEN triggers](https://docs.github.com/en/actions/concepts/security/github_token) ·
[bot PRs need approval (2026-06-11)](https://github.blog/changelog/2026-06-11-bot-created-pull-requests-can-run-workflows-if-approved/) ·
[self-hosted fee postponed](https://github.blog/changelog/2025-12-16-coming-soon-simpler-pricing-and-a-better-experience-for-github-actions/)

Runner providers: [Namespace pricing](https://namespace.so/pricing) ·
[Namespace Rust case study](https://namespace.so/blog/reducing-github-actions-runtime-for-a-rust-project) ·
[Blacksmith](https://www.blacksmith.sh/pricing) · [Depot](https://depot.dev/pricing) ·
[WarpBuild](https://www.warpbuild.com/pricing) · [RunsOn](https://runs-on.com/pricing/) ·
[RunsOn benchmarks](https://runs-on.com/benchmarks/) ·
[BuildJet shutdown](https://www.warpbuild.com/blog/buildjet-shutting-down) ·
[Cirrus Labs joins OpenAI](https://macstadium.com/blog/cirrus-labs-is-joining-openai)

Buildkite: [pricing](https://buildkite.com/pricing) ·
[hosted macOS](https://buildkite.com/docs/agent/buildkite-hosted/macos) ·
[hosted Windows](https://buildkite.com/docs/agent/buildkite-hosted/windows) ·
[GitHub integration](https://buildkite.com/docs/pipelines/source-control/github) ·
[notify / GitHub Checks](https://buildkite.com/docs/pipelines/configure/notify) ·
[merge queue](https://buildkite.com/docs/tutorials/github-merge-queue) ·
[dynamic pipelines](https://buildkite.com/docs/pipelines/configure/dynamic-pipelines) ·
[Windows agent](https://buildkite.com/docs/agent/self-hosted/install/windows)

Other: [CircleCI pricing](https://circleci.com/pricing/) · [Tart licensing](https://tart.run/licensing/) ·
[Scaleway Apple silicon](https://www.scaleway.com/en/pricing/apple-silicon/) ·
[AWS EC2 Mac](https://aws.amazon.com/ec2/instance-types/mac/) ·
[setup-dev-drive](https://github.com/samypr100/setup-dev-drive) · [cargo-xwin](https://github.com/rust-cross/cargo-xwin) ·
[M6 Mac mini](https://macworld.com/article/3220063/apple-launches-new-m6-mac-mini-with-another-price-hike.html)
