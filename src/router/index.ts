import { createRouter, createWebHistory } from 'vue-router';
import InboxView from '@/views/InboxView.vue';
import { MAIL_LIST_VIEWS, selectMailListView } from './mailViews';

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
    // Named lists are scopes of the inbox route; their paths are deep links.
    ...MAIL_LIST_VIEWS.filter((view) => view !== 'inbox').map((view) => ({
      path: `/${view}`,
      name: view,
      redirect: () => {
        selectMailListView(view);
        return { name: 'inbox' };
      },
    })),
    // Waiting for a reply lives in Later now.
    { path: '/follow-ups', name: 'followups', redirect: { name: 'later' } },
    { path: '/contacts', name: 'contacts', component: ContactsView },
    { path: '/new-senders', name: 'new-senders', component: NewSendersView },
    { path: '/topics/:id', name: 'topic', component: TopicView },
    { path: '/search', name: 'search', component: SearchView },
    { path: '/settings', name: 'settings', component: SettingsView },
  ],
});
