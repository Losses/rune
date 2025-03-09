import { writable, derived } from 'svelte/store';
import { browser } from '$app/environment';

const createTokenStore = () => {
	const initialValue = browser ? localStorage.getItem('token') || '' : '';

	const { subscribe, set } = writable<string>(initialValue);

	if (browser) {
		subscribe((value) => {
			if (value) {
				localStorage.setItem('token', value);
			} else {
				localStorage.removeItem('token');
			}
		});
	}

	return {
		subscribe,
		setToken: (newToken: string) => set(newToken),
		clearToken: () => set('')
	};
};

export const token = createTokenStore();

export const isAuthenticated = derived(token, ($token) => Boolean($token));
