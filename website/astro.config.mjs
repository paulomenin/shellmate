// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import mermaid from 'astro-mermaid';

// Landing page at `/`, documentation under `/docs`. Served from the root of a domain.
//   SITE=https://example.org pnpm build     (the public URL: canonical links and the sitemap need it)
// To serve from a sub-path instead (for example GitHub Pages without a custom domain), also set BASE=/shellmate
const site = process.env.SITE || undefined;
const base = process.env.BASE || '/';

export default defineConfig({
	site,
	base,
	integrations: [
		// before starlight; follows the site's light/dark theme. Mermaid is loaded only on pages that have a diagram.
		mermaid({ autoTheme: true }),
		starlight({
			title: 'shellmate',
			description:
				'A companion for your shell: ask, generate and explain commands from your prompt in fish, zsh and bash. Suggestions land in your input line and are never run for you.',
			favicon: '/favicon.svg',
			// Open external links (content, the GitHub icon, the edit link) in a new tab.
			head: [
				// Fold-away side panels: sets the saved state before the page paints (see public/layout-toggle.js).
				{ tag: 'script', attrs: { src: `${base.replace(/\/$/, '')}/layout-toggle.js` } },
				{
					tag: 'script',
					content: `document.addEventListener('DOMContentLoaded', () => { for (const a of document.querySelectorAll('a[href^="http"]')) { if (a.hostname !== location.hostname) { a.target = '_blank'; a.rel = 'noopener noreferrer'; } } });`,
				},
			],
			logo: {
				light: './src/assets/logo-header.svg',
				dark: './src/assets/logo-header-dark.svg',
				replacesTitle: true,
			},
			customCss: ['./src/styles/custom.css'],
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/paulomenin/shellmate' },
			],
			editLink: {
				baseUrl: 'https://github.com/paulomenin/shellmate/edit/main/website/',
			},
			lastUpdated: false,
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'Install', slug: 'docs/start/install' },
						{ label: 'Quick start', slug: 'docs/start/quick-start' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Using shellmate', slug: 'docs/guides/using-shellmate' },
						{ label: 'Follow-up conversations', slug: 'docs/guides/conversations' },
						{ label: 'Providers', slug: 'docs/guides/providers' },
						{ label: 'Choosing a local model', slug: 'docs/guides/local-models' },
						{ label: 'Privacy and redaction', slug: 'docs/guides/privacy' },
						{ label: 'The man tool', slug: 'docs/guides/man-tool' },
						{ label: 'Letting the model run commands', slug: 'docs/guides/exec-tool' },
						{ label: 'Letting the model read files', slug: 'docs/guides/read-tool' },
						{ label: "Asking about a command's output", slug: 'docs/guides/capture' },
						{ label: 'Checking suggested commands', slug: 'docs/guides/validator' },
						{ label: 'Colours and formatting', slug: 'docs/guides/output' },
						{ label: 'Shell integration', slug: 'docs/guides/shells' },
						{ label: 'Audit log and --explain', slug: 'docs/guides/audit' },
						{ label: 'Troubleshooting', slug: 'docs/guides/troubleshooting' },
					],
				},
				{
					label: 'Benchmarks',
					items: [
						{ label: 'Overview', slug: 'docs/benchmarks' },
						{ label: 'Performance', slug: 'docs/benchmarks/performance' },
						{ label: 'Results', slug: 'docs/benchmarks/results' },
						{ label: 'Chat models', slug: 'docs/benchmarks/chat' },
						{ label: 'Decision models', slug: 'docs/benchmarks/decision' },
						{ label: 'Composed', slug: 'docs/benchmarks/composed' },
						{ label: 'Cases', slug: 'docs/benchmarks/cases' },
						{ label: 'Evaluator', slug: 'docs/benchmarks/evaluation' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Configuration', slug: 'docs/reference/configuration' },
						{ label: 'Command line', slug: 'docs/reference/cli' },
					],
				},
				{
					label: 'Concepts',
					items: [
						{ label: 'Security model', slug: 'docs/concepts/security-model' },
						{ label: 'Architecture', slug: 'docs/concepts/architecture' },
						{ label: 'Life of a request', slug: 'docs/concepts/request-lifecycle' },
						{ label: 'Plugins', slug: 'docs/concepts/plugins' },
						{ label: 'The LLM pipeline', slug: 'docs/concepts/llm-pipeline' },
						{ label: 'Tool calling', slug: 'docs/concepts/tool-calling' },
						{ label: 'Context and prompts', slug: 'docs/concepts/context-and-prompts' },
						{ label: 'Command checks', slug: 'docs/concepts/command-checks' },
						{ label: 'State on disk', slug: 'docs/concepts/state-and-storage' },
						{ label: 'Output and presentation', slug: 'docs/concepts/output-and-presentation' },
						{ label: 'Shell glue', slug: 'docs/concepts/glue-protocol' },
						{ label: 'Module guide', slug: 'docs/concepts/module-guide' },
						{ label: 'Status and roadmap', slug: 'docs/concepts/roadmap' },
					],
				},
				{
					label: 'Contributing',
					items: [
						{ label: 'Contributing', slug: 'docs/contributing' },
						{ label: 'Development requirements', slug: 'docs/development-requirements' },
					],
				},
			],
		}),
	],
});
