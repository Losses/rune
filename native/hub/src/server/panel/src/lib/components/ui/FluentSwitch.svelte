<script lang="ts">
	interface Props {
		label: string;
		checked: boolean;
		onChange: (x: boolean) => void;
		id?: string;
	}

	const { label, id, checked, onChange }: Props = $props();

	$effect(() => {
		input.checked = checked;
	});

	const handleChange = (e: Event) => {
		onChange((e.target as HTMLInputElement).checked);
	};

	let input: HTMLInputElement;
</script>

<fluent-field label-position="after">
	<!-- svelte-ignore a11y_label_has_associated_control -->
	<label slot="label">{label}</label>
	<fluent-switch {id} {checked} onchange={handleChange} bind:this={input} slot="input"
	></fluent-switch>
</fluent-field>
