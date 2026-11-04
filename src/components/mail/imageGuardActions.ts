import type { Component, InjectionKey } from 'vue';

/**
 * The phone's Show / ⋯ pair for EmailRenderer's blocked-images notice
 * (ImageGuardPhoneActions.vue), handed in by the app shell rather than
 * imported by the renderer: the message-file viewer renders the same notice,
 * is never on a phone, and stays inside its bundle budget without it. Without
 * a provider the renderer keeps the inline links.
 */
export const IMAGE_GUARD_PHONE_ACTIONS: InjectionKey<Component> = Symbol('imageGuardPhoneActions');
