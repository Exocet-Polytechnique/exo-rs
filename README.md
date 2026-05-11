# exo-rs
Programs running on our electrical control units (ECUs)

## Main branch

The `main` branch represents the **stable** state of the codebase.

- Only tested and/or validated code should be merged here
- Changes should go through a pull request (unless an urgent hotfix)
- Each merge should be tagged with a version number (e.g. `v1.2.0`)
- Code merged here is considered ready to be flashed onto the physical system
- If it breaks the build or causes a regression, it should not belong here

When in doubt, keep it in `develop` until it's been validated.
