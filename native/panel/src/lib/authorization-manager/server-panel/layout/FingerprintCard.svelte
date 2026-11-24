<script lang="ts">
	import Card from '$lib/components/ui/Card.svelte';
	import FingerprintIcon from '$lib/components/icons/FingerprintIcon.svelte';
	import FluentButton from '$lib/components/ui/FluentButton.svelte';

	interface Props {
		onClick?: () => void;
	}

	let { onClick }: Props = $props();
</script>

<Card noPadding>
	<FluentButton appearance="primary" shape="square" fullWidth fullHeight {onClick}>
		<div class="reveal-fingerprint-tag">
			<FingerprintIcon />
			<div class="label">
				<fluent-text size="500" class="reveal" weight="bold">Reveal</fluent-text>
				<fluent-text size="200">Device Fingerprint</fluent-text>
			</div>
		</div>
	</FluentButton>
</Card>

<style>
	.reveal-fingerprint-tag {
		padding: 0 28px;
		display: flex;
		flex-direction: column;
		justify-content: center;
		align-items: center;
		font-size: 48px;
	}

	.reveal-fingerprint-tag .label {
		margin-top: 12px;
	}

	.reveal-fingerprint-tag .label fluent-text {
		display: block;
		text-align: center;
	}
</style>
