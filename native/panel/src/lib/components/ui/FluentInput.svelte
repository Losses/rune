<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		value: string;
		id?: string;
		appearance?: 'filled-lighter' | 'outline';
		type?: 'email' | 'password' | 'tel' | 'text' | 'url';
		onChange: (x: string) => void;
		children: Snippet;
		end?: Snippet;
	}

	const { id, appearance, value, type, children, end, onChange }: Props = $props();

	$effect(() => {
		input.value = value;
	});

	const handleInput = (e: Event) => {
		onChange((e.target as HTMLInputElement).value);
	};

	let input: HTMLInputElement;
</script>

<fluent-text-input {id} {appearance} {value} {type} oninput={handleInput} bind:this={input}>
	{@render children()}
	<span slot="end">{@render end?.()}</span>
</fluent-text-input>
