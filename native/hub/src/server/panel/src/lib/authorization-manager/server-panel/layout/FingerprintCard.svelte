<script lang="ts">
	import Card from '$lib/components/ui/Card.svelte';
	import FingerprintIcon from '$lib/components/icons/FingerprintIcon.svelte';
</script>

<Card noPadding>
	<fluent-button appearance="primary" shape="square" class="reveal-fingerprint-button">
		<div class="reveal-fingerprint-tag">
			<FingerprintIcon />
			<div class="label">
				<fluent-text size="500" class="reveal" weight="bold">Reveal</fluent-text>
				<fluent-text size="200">Device Fingerprint</fluent-text>
			</div>
		</div>
	</fluent-button>
</Card>

<style>
	.reveal-fingerprint-button {
		width: 100%;
		height: 100%;
		padding: 0 28px;
	}

	.reveal-fingerprint-tag {
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
