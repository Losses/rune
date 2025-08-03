<script lang="ts">
	import { onMount } from 'svelte';

	import { browser } from '$app/environment';

	import { webLightTheme } from '@fluentui/tokens';
	import { setTheme } from '@fluentui/web-components';
	import '@fluentui/web-components/dialog-body.js';
	import '@fluentui/web-components/menu-button.js';
	import '@fluentui/web-components/text-input.js';
	import '@fluentui/web-components/menu-list.js';
	import '@fluentui/web-components/menu-item.js';
	import '@fluentui/web-components/dropdown.js';
	import '@fluentui/web-components/listbox.js';
	import '@fluentui/web-components/spinner.js';
	import '@fluentui/web-components/button.js';
	import '@fluentui/web-components/option.js';
	import '@fluentui/web-components/switch.js';
	import '@fluentui/web-components/dialog.js';
	import '@fluentui/web-components/badge.js';
	import '@fluentui/web-components/field.js';
	import '@fluentui/web-components/menu.js';
	import '@fluentui/web-components/text.js';
	import { register as registerElevation } from '@ax-design/elevation';

	import '../app.css';

	let { children } = $props();
	onMount(() => {
		if (browser) {
			setTheme(webLightTheme);
			registerElevation();
		}
	});
</script>

{@render children()}
