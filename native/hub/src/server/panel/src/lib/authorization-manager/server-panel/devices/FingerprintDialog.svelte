<script lang="ts">
	import FingerprintFigure from './FingerprintFigure.svelte';

	interface Props {
		fingerprint: string;
	}

	let { fingerprint }: Props = $props();

	let dialog: HTMLDialogElement;

	export function show() {
		dialog.show();
	}
</script>

<fluent-dialog bind:this={dialog}>
	<fluent-dialog-body>
		<FingerprintFigure {fingerprint} />

		<div slot="title">Fingerprint</div>
	</fluent-dialog-body>
</fluent-dialog>
