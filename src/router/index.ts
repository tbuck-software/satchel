import { createRouter, createWebHistory } from 'vue-router';
import InboxView from '@/views/InboxView.vue';

// Inbox is the default route and stays in the startup bundle. Every secondary
// workspace is loaded on first navigation so settings, calendar and composer
// code do not compete with the mailbox for parse and evaluation time.
const ThreadDetailView = () => import('@/views/ThreadDetailView.vue');
const ComposeView = () => import('@/views/ComposeView.vue');
const SearchView = () => import('@/views/SearchView.vue');
const SettingsView = () => import('@/views/SettingsView.vue');
const DraftsView = () => import('@/views/DraftsView.vue');
const OutboxView = () => import('@/views/OutboxView.vue');
const CalendarView = () => import('@/views/CalendarView.vue');
const FollowUpsView = () => import('@/views/FollowUpsView.vue');
const ContactsView = () => import('@/views/ContactsView.vue');
const NewSendersView = () => import('@/views/NewSendersView.vue');
const TopicView = () => import('@/views/TopicView.vue');

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/inbox' },
    { path: '/inbox', name: 'inbox', component: InboxView },
    { path: '/thread/:id', name: 'thread', component: ThreadDetailView },
    { path: '/compose', name: 'compose', component: ComposeView },
    { path: '/drafts', name: 'drafts', component: DraftsView },
    { path: '/outbox', name: 'outbox', component: OutboxView },
    { path: '/calendar', name: 'calendar', component: CalendarView },
    { path: '/follow-ups', name: 'followups', component: FollowUpsView },
    { path: '/contacts', name: 'contacts', component: ContactsView },
    { path: '/new-senders', name: 'new-senders', component: NewSendersView },
    { path: '/topics/:id', name: 'topic', component: TopicView },
    { path: '/search', name: 'search', component: SearchView },
    { path: '/settings', name: 'settings', component: SettingsView },
  ],
});
