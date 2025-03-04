<script lang="ts">
	import { format } from 'timeago.js';
	import StatusBadge from './StatusBadge.svelte';
	import DeviceStatusMenu from './DeviceStatusMenu.svelte';
	import type { IDevice } from '$lib/authorization-manager';
	import FingerprintIcon from '$lib/icons/FingerprintIcon.svelte';
	import FingerprintFigure from './FingerprintFigure.svelte';

	interface Props {
		device: IDevice;
		onUpdateStatus: (deviceId: string, status: string) => void;
	}

	let { device, onUpdateStatus }: Props = $props();

	let dialog: HTMLDialogElement;
</script>

<tr>
	<th>
		<fluent-text weight="bold">{device.name}</fluent-text>
		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<fluent-button
			icon-only
			size="small"
			class="fingerprint-button"
			aria-label="Display Fingerprints"
			onclick={() => dialog.show()}
			><span class="fingerprint-icon"><FingerprintIcon /></span></fluent-button
		>
		<fluent-dialog id={`dialog-${device.fingerprint}`} bind:this={dialog}>
			<fluent-dialog-body>
				<FingerprintFigure fingerprint={device.fingerprint} />

				<div slot="title">Fingerprint</div>
			</fluent-dialog-body>
		</fluent-dialog>
	</th>
	<th>
		<StatusBadge status={device.status} />
	</th>
	<th><fluent-text>{format(device.lastSeen, 'en_US')}</fluent-text></th>
	<th>
		<DeviceStatusMenu deviceId={device.id} {onUpdateStatus} variant="small" />
	</th>
</tr>

<style>
	th {
		padding: 16px;
		text-align: start;
		vertical-align: center;
	}

	tr {
		border-bottom: 1px solid #e4e4e4;
		transition: background-color 100ms;
	}

	tr:hover {
		background-color: #fafafa;
	}

	.fingerprint-button {
		margin-left: 8px;
	}

	.fingerprint-icon {
		display: contents;
		color: rgba(0, 0, 0, 0.75);
	}
</style>
