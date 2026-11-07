import { startRecipientDrag } from './appDragTarget';

/*
 * Contacts dragged onto the composer: each arrives as a recipient. Apart from
 * `appDrag.ts` because no drag source in the startup bundle carries one.
 */

export interface DraggedContact {
  email: string;
  name?: string | null;
}

/** The chip value a contact becomes: `Name <address>`, or the bare address. */
export function contactRecipient(contact: DraggedContact): string {
  const name = contact.name?.trim();
  const email = contact.email.trim();
  if (!name || name.toLowerCase() === email.toLowerCase()) return email;
  return `${name} <${email}>`;
}

export function startContactDrag(event: DragEvent, contacts: DraggedContact[]) {
  const addresses = contacts.filter((contact) => contact.email.trim()).map(contactRecipient);
  if (addresses.length === 0) return;
  startRecipientDrag(event, addresses, null);
}
