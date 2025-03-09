<script lang="ts">
	import { onMount } from 'svelte';
	import { get } from 'svelte/store';

	import { isAuthenticated, token } from '$lib/stores/token.svelte';

	import LoginPanel from './login-panel/LoginPanel.svelte';
	import ServerPanel from './server-panel/ServerPanel.svelte';
	import SpinnerScreen from './spinner-screen/SpinnerScreen.svelte';

	import type { IDevice, IServerConfig, IUserSummaryResponse } from '.';

	let isRefreshing = $state(true);
	let initComplete = $state(false);
	let serverConfig: IServerConfig = $state({
		alias: '',
		broadcastEnabled: false
	});

	interface IPanelSelfResponse {
		fingerprint: string;
		alias: string;
		broadcasting: boolean;
	}

	let devices: IDevice[] = $state([]);

	const isAuthenticated$ = $derived(isAuthenticated);

	$effect(() => {
		let interval: number;
		if ($isAuthenticated$) {
			const fetchDevices = async () => {
				try {
					const response = await fetch('/panel/users', {
						headers: {
							Authorization: `Bearer ${get(token)}`
						}
					});

					if (!response.ok) {
						if (response.status === 401) {
							token.clearToken();
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

	async function fetchServerConfig() {
		try {
			const res = await fetch('/panel/self', {
				headers: {
					Authorization: `Bearer ${get(token)}`
				}
			});
			const config: IPanelSelfResponse = await res.json();
			serverConfig.alias = config.alias;
			serverConfig.broadcastEnabled = config.broadcasting;
		} catch (error) {
			console.error('Failed to fetch server config:', error);
		}
	}

	$effect(() => {
		if ($isAuthenticated$) {
			fetchServerConfig();
		}
	});

	onMount(async () => {
		if (get(isAuthenticated$)) {
			try {
				const response = await fetch('/panel/auth/refresh', {
					method: 'POST',
					headers: {
						Authorization: `Bearer ${get(token)}`,
						'Content-Type': 'application/json'
					}
				});

				if (!response.ok) {
					const error = await response.text();
					throw new Error(error || 'Token refresh failed');
				}

				const { token: newToken } = await response.json();
				token.setToken(newToken);
			} catch (error) {
				token.clearToken();
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
			token.setToken(authToken);
		} catch (error) {
			console.error('Login error:', error);
			alert(error instanceof Error ? error.message : 'Login failed. Please try again.');
		}
	};

	const onServerConfigUpdate = (config: IServerConfig) => {
		serverConfig = config;
	};

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
		{#if !$isAuthenticated$}
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
