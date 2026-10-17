import type { ContactDto as WireContactDto } from '../services/backend/generated/app/commands/contacts/ContactDto';
import type { OptionalWireFields } from '../services/backend/wireTypes';
export type BackendContact = OptionalWireFields<WireContactDto, 'display_name' | 'avatar_url' | 'avatar_source' | 'avatar_updated_at' | 'last_contacted' | 'notes'>;
