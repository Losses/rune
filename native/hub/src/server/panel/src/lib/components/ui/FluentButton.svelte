<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		disabled?: boolean;
		disabledFocusable?: boolean;
		appearance?: 'primary' | 'outline' | 'subtle' | 'transparent';
		formaction?: string;
		form?: string;
		formenctype?: string;
		formmethod?: string;
		formnovalidate?: boolean;
		formtarget?: string;
		iconOnly?: boolean;
		name?: string;
		size?: 'small' | 'medium' | 'large';
		shape?: 'circular' | 'rounded' | 'square';
		type?: 'submit' | 'reset' | 'button';
		value?: string;
		children: Snippet;
		start?: Snippet;
		end?: Snippet;
		onClick?: (e: MouseEvent) => void;
	}

	const {
		disabled,
		disabledFocusable,
		appearance,
		formaction,
		form,
		formenctype,
		formmethod,
		formnovalidate,
		formtarget,
		iconOnly,
		name,
		size,
		shape,
		type,
		value,
		children,
		start,
		end,
		onClick
	}: Props = $props();

	const handleClick = (e: MouseEvent) => {
		onClick?.(e);
	};

	let button: HTMLButtonElement;
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<fluent-button
	{disabled}
	disabled-focusable={disabledFocusable}
	{appearance}
	{formaction}
	{form}
	{formenctype}
	{formmethod}
	{formnovalidate}
	{formtarget}
	icon-only={iconOnly}
	{name}
	{size}
	{shape}
	{type}
	{value}
	onclick={handleClick}
	bind:this={button}
>
	{#if start}
		<span slot="start">{@render start()}</span>
	{/if}
	{@render children()}
	{#if end}
		<span slot="end">{@render end()}</span>
	{/if}
</fluent-button>
