<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		children: Snippet;
		position: {
			bottom?: string;
			right?: string;
			left?: string;
			top?: string;
		};
		onClick?: () => void;
	}

	let { children, position, onClick }: Props = $props();
</script>

<button
	class="fab"
	onclick={onClick}
	style:bottom={position.bottom}
	style:right={position.right}
	style:left={position.left}
	style:top={position.top}
>
	<span class="material-icons">{@render children()}</span>
</button>

<style>
	.fab {
		width: 48px;
		height: 48px;
		font-size: 20px;
		line-height: 0;
		border-radius: 100%;
		background: var(--colorBrandBackground);
		position: fixed;
		display: flex;
		justify-content: center;
		align-items: center;
		color: white;
		border: none;
		z-index: 1000;

		--elevation-depth: 8;
		transition: box-shadow 200ms;
		box-shadow:
			0 calc(0.4 * var(--elevation-depth, 0) * 1px) calc(0.9 * var(--elevation-depth, 0) * 1px) 0
				rgba(
					0,
					0,
					0,
					calc(
						0.13441 - 0.00063 * var(--elevation-depth, 0) + 0.00003 * var(--elevation-depth, 0) *
							var(--elevation-depth, 0)
					)
				),
			0 calc(0.075 * var(--elevation-depth, 0) * 1px) calc(0.225 * var(--elevation-depth, 0) * 1px)
				0
				rgba(
					0,
					0,
					0,
					calc(
						0.10997 - 0.00051 * var(--elevation-depth, 0) + 0.00003 * var(--elevation-depth, 0) *
							var(--elevation-depth, 0)
					)
				);
	}

	.fab:hover {
		--elevation-depth: 12;
	}

	.fab:active {
		--elevation-depth: 20;
	}
</style>
