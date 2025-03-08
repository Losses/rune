<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		value: string;
		onChange: (x: string) => void;
		id?: string;
		appearance?: 'filled-lighter' | 'outline';
		children: Snippet;
	}

	const { id, appearance, value, children, onChange }: Props = $props();

	$effect(() => {
		input.value = value;
	});

	const handleInput = (e: Event) => {
		onChange((e.target as HTMLInputElement).value);
	};

	let input: HTMLInputElement;
</script>

<fluent-text-input {id} {appearance} {value} oninput={handleInput} bind:this={input}>
	{@render children()}
</fluent-text-input>
