# CI runner boundaries

All pull requests, including forks, run on GitHub-hosted machines. No workflow
is attached to a developer workstation or a persistent personal account.
Frontend checks use Ubuntu, backend tests use macOS, and the native portability
matrix checks Linux and Windows. No secrets are available to fork builds.

Release jobs accept only `main` of `tbuck-software/satchel`. Build jobs have
read-only repository access; only the final publishing job can write releases.
Updater signing keys are repository secrets. Mobile signing remains separate.

Migration validation, dependency audits, secret scanning, Android builds, iOS
simulator builds, and parser fuzzing run on GitHub-hosted machines too.
Mobile workflows read the pinned toolchain and public postroom dependencies.

The clean repository starts with a new root commit. The old development
history, issues, screenshots, logs, and private backlog remain in the separate
private archive and must never be merged or mirrored into this repository.
