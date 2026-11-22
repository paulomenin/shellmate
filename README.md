# shellmate

shellmate is an open-source command-line companion for your shell (fish, zsh, and bash). Type a question at your shell prompt and press a shortcut to get a streaming answer. When shellmate suggests a command, it puts it in your input line so you can review, edit, and run it yourself.

See the [documentation on GitHub Pages](https://paulomenin.github.io/shellmate/) for installation, configuration, and usage guides.

## Features

- **Ask, generate, and explain.** Press `Alt+M` to ask a question, `Alt+I` to turn a request into a command, or `Alt+J` to explain a command you typed.
- **Follow-up conversations.** Ask “same, but only .log files” in the same shell session. Conversation history is stored locally.
- **Local and hosted models.** Connect to an OpenAI-compatible provider, such as Ollama or OpenRouter.
- **Privacy controls.** Choose how much shell context to share. Requests to remote providers require active secret redaction.
- **Optional tools.** Let the model look up manual pages, read files, or request command execution. Command execution is subject to approval rules.
- **Setup and diagnostics.** Configure a provider and shell shortcuts with `setup`, check your installation with `doctor`, and inspect activity with the audit log.

## Quick start

From the repository root, build from source and run setup.

```sh
cargo build --release
target/release/shellmate setup
target/release/shellmate doctor
```

Open a new shell, type `how do I find the biggest files here`, and press `Alt+M`.

## Contributing

See the [contributing guide](website/src/content/docs/docs/contributing.md) for development setup and checks.

## License

[MIT](LICENSE)
