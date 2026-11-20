// Collapsible side panels (navigation on the left, "On this page" on the right) for wide screens.
// The choice is remembered per browser and applied before the page paints, so it does not flash.
(function () {
	var root = document.documentElement;
	var PANELS = [
		{ name: 'sidebar', needs: 'data-has-sidebar', label: 'navigation' },
		{ name: 'toc', needs: 'data-has-toc', label: 'table of contents' },
	];
	function stored(name) {
		try {
			return localStorage.getItem('sm-hide-' + name) === '1';
		} catch (e) {
			return false;
		}
	}
	function store(name, hidden) {
		try {
			localStorage.setItem('sm-hide-' + name, hidden ? '1' : '0');
		} catch (e) {}
	}
	PANELS.forEach(function (p) {
		if (stored(p.name)) root.setAttribute('data-sm-hide-' + p.name, '');
	});
	document.addEventListener('DOMContentLoaded', function () {
		PANELS.forEach(function (p) {
			if (!root.hasAttribute(p.needs)) return;
			var button = document.createElement('button');
			button.type = 'button';
			button.className = 'sm-panel-toggle';
			button.setAttribute('data-panel', p.name);
			function show() {
				var hidden = root.hasAttribute('data-sm-hide-' + p.name);
				var text = (hidden ? 'Show ' : 'Hide ') + p.label;
				button.setAttribute('aria-label', text);
				button.setAttribute('aria-expanded', hidden ? 'false' : 'true');
				button.title = text;
			}
			button.addEventListener('click', function () {
				var hidden = !root.hasAttribute('data-sm-hide-' + p.name);
				if (hidden) root.setAttribute('data-sm-hide-' + p.name, '');
				else root.removeAttribute('data-sm-hide-' + p.name);
				store(p.name, hidden);
				show();
			});
			show();
			document.body.appendChild(button);
		});
	});
})();
