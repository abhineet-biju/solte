## Code
- Keep modules focused and comments brief.
- Run formatting, relevant tests, and Clippy before committing completed changes.
- Keep network and disk operations out of UI rendering.
- Mouse and keyboard interactions must trigger the same application actions.

## Local installation
- After completing application changes and validation, replace the locally installed `solte` executable with the current optimized build so the user can run `solte` directly.
- Resolve the installation with `command -v solte`; it is currently `~/.local/bin/solte`. Replace the executable atomically and verify the installed command and terminal workflow.
- This is a standing user instruction. Do not request confirmation for each local binary update.

## Git commits
- Commit meaningful features and subfeatures as they are completed.
- Use conventional commit subjects: `type: summary` or `type(scope): summary`.
- For a longer message, use a blank line followed by body bullets with real newlines.
