.PHONY: build test lint fmt clean release install install-skill install-fastedit install-full

build:
	cargo build --release

test:
	cargo test -p tldr-core --lib
	cargo test -p tldr-cli --lib

lint:
	cargo clippy --workspace -- -D warnings

fmt:
	cargo fmt --check

clean:
	cargo clean

install: build
	cp target/release/tldr ~/.local/bin/tldr

# Install the agent skill from THIS checkout, via the open skills CLI
# (https://github.com/vercel-labs/skills). The local-path source is deliberate: it installs the
# skill that matches the binary you just built, so an agent never reads docs for a tldr you do
# not have. `skills add` writes to every supported agent it finds (Claude Code, Codex, Cursor,
# OpenCode, …) and is idempotent, so re-running is safe.
# `-g` is load-bearing: without it `skills add` installs project-level, i.e. into THIS repo's
# .claude/skills — the one project where nobody needs it. The skill documents a binary that lives
# on $PATH, so it belongs at user level alongside it. `--all` = every skill, every detected agent,
# no prompts, which is what makes the target usable from `install-full` and safe to re-run.
install-skill:
	npx --yes skills add -g --all ./skills/tldr-code

# fastedit — the AST-scoped WRITE companion (https://github.com/parcadei/fastedit). tldr READS
# code; fastedit EDITS it by symbol name, so an agent never repeats old lines to say where an
# edit goes. Optional on purpose, and it is NOT a cargo dependency: fastedit is a Python package,
# and `cargo install` has no post-install hook — a build.rs that reached the network would fire
# during CI and docs.rs builds. So the bundle lives here, where running it is a choice.
#
# NOTE ON DIRECTION: fastedit's own README lists tldr as ITS prerequisite, not the reverse. This
# target is a convenience bundle for the pair, never a requirement of tldr.
#
# The ~3 GB merge model is deliberately NOT pulled here — a `make install` that silently
# downloads gigabytes is a bad neighbour. The command is printed instead.
install-fastedit:
	@if command -v fastedit >/dev/null 2>&1; then \
		echo "fastedit already installed: $$(command -v fastedit)"; \
	elif ! command -v uv >/dev/null 2>&1; then \
		echo "uv not found — install it first (https://docs.astral.sh/uv/), then re-run 'make install-fastedit'"; \
		exit 1; \
	else \
		if [ "$$(uname -s)" = "Darwin" ] && [ "$$(uname -m)" = "arm64" ]; then \
			extras='mlx,mcp'; model='mlx-8bit'; \
		else \
			extras='mcp'; model='bf16'; \
		fi; \
		echo "installing fastedits[$$extras]"; \
		uv tool install "fastedits[$$extras]"; \
		echo ""; \
		echo "Next (one-time, ~3 GB): fastedit pull --model $$model"; \
	fi

# tldr + the skill + the WRITE companion, in one go.
install-full: install install-fastedit install-skill

# Run all checks (CI equivalent)
check: fmt lint test

# Quick dev build
dev:
	cargo build
