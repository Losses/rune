<script lang="ts">
	import Card from '$lib/components/ui/Card.svelte';
	import FluentInput from '$lib/components/ui/FluentInput.svelte';
	import FluentButton from '$lib/components/ui/FluentButton.svelte';

	interface Props {
		onSubmit: (password: string) => void;
	}

	let { onSubmit }: Props = $props();
	let password = $state('');

	const handlePasswordChange = (value: string) => {
		password = value;
	};

	const handleSubmit = () => {
		onSubmit(password);
		password = '';
	};

	const handleCancel = () => {
		password = '';
	};
</script>

<Card headerText="Login" headerSubtitle="Enter your password to access the system">
	<div>
		<FluentInput
			type="password"
			appearance="filled-lighter"
			value={password}
			onChange={handlePasswordChange}
		>
			Password
		</FluentInput>
	</div>

	<div class="button-group">
		<FluentButton appearance="primary" onClick={handleSubmit}>Login</FluentButton>
	</div>
</Card>

<style>
	.button-group {
		display: flex;
		gap: 10px;
		justify-content: flex-end;
		padding-top: 20px;
	}
</style>
