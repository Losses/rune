<script lang="ts">
	import { localStore } from '$lib/utils.svelte';
	import { onMount } from 'svelte';

	import LoginPanel from './login-panel/LoginPanel.svelte';
	import ServerPanel from './server-panel/ServerPanel.svelte';
	import SpinnerScreen from './spinner-screen/SpinnerScreen.svelte';

	import type { IDevice, IServerConfig, IUserSummaryResponse } from '.';

	let token = localStore<string>('token', '');
	let isRefreshing = $state(true);
	let initComplete = $state(false);
	let serverConfig: IServerConfig = $state({
		alias: 'Main Server',
		broadcastEnabled: true
	});

	let devices: IDevice[] = $state([]);

	$effect(() => {
		let interval: number;

		if (token.value) {
			const fetchDevices = async () => {
				try {
					const response = await fetch('/panel/users', {
						headers: {
							Authorization: `Bearer ${token.value}`
						}
					});

					if (!response.ok) {
						if (response.status === 401) {
							token.value = '';
						}
						throw new Error('Failed to fetch devices');
					}

					const users = await response.json();
					devices = users.map((user: IUserSummaryResponse) => ({
						id: user.fingerprint,
						name: user.alias,
						fingerprint: user.fingerprint,
						status: user.status,
						lastSeen: new Date(user.add_time.secs_since_epoch * 1000)
					}));
				} catch (error) {
					console.error('Device polling error:', error);
				}
			};

			fetchDevices();
			interval = setInterval(fetchDevices, 3000);
		}

		return () => {
			if (interval) clearInterval(interval);
		};
	});

	onMount(async () => {
		if (token.value) {
			try {
				const response = await fetch('/panel/auth/refresh', {
					method: 'POST',
					headers: {
						Authorization: `Bearer ${token.value}`,
						'Content-Type': 'application/json'
					}
				});

				if (!response.ok) {
					const error = await response.text();
					throw new Error(error || 'Token refresh failed');
				}

				const { token: newToken } = await response.json();
				token.value = newToken;
			} catch (error) {
				token.value = '';
			}
		}
		isRefreshing = false;
		initComplete = true;
	});

	const handleLogin = async (password: string) => {
		try {
			const response = await fetch('/panel/auth/login', {
				method: 'POST',
				headers: {
					'Content-Type': 'application/json'
				},
				body: JSON.stringify({ password })
			});

			if (!response.ok) {
				const error = await response.json();
				throw new Error(error.message || 'Login failed');
			}

			const { token: authToken } = await response.json();
			token.value = authToken;
		} catch (error) {
			console.error('Login error:', error);
			alert(error || 'Login failed. Please try again.');
		}
	};

	/** Handle server config updates */
	const onServerConfigUpdate = (config: IServerConfig) => {
		serverConfig = config;
	};

	/** Handle device status updates */
	const onDeviceStatusUpdate = (deviceId: string, newStatus: string) => {
		devices = devices.map((device) =>
			device.id === deviceId ? { ...device, status: newStatus } : device
		);
	};
</script>

{#if isRefreshing}
	<SpinnerScreen />
{:else if initComplete}
	<main>
		{#if !token.value}
			<LoginPanel onSubmit={handleLogin} />
		{:else}
			<ServerPanel
				{serverConfig}
				{devices}
				onUpdateConfig={onServerConfigUpdate}
				onUpdateDeviceStatus={onDeviceStatusUpdate}
			/>
		{/if}
	</main>
{/if}
