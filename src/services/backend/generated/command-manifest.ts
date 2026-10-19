// Generated from Rust command signatures. Run pnpm ipc:generate.
export default {
    "commands": {
        "abort_oauth_flow": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::oauth::AbortOAuthInput"
                    }
                }
            }
        },
        "acknowledge_app_update_flush": {
            "kind": "object",
            "fields": {
                "requestId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "saved": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "acknowledge_draft_flush": {
            "kind": "object",
            "fields": {
                "saved": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "act_on_tray_thread": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::tray::TrayThreadActionInput"
                    }
                }
            }
        },
        "add_account": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::accounts::NewAccount"
                    }
                }
            }
        },
        "add_smime_certificate": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::smime::AddSmimeCertificateInput"
                    }
                }
            }
        },
        "add_topic_file": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::topics::TopicFileInput"
                    }
                }
            }
        },
        "add_topic_items": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::topics::AddTopicItemsInput"
                    }
                }
            }
        },
        "app_build_info": {
            "kind": "object",
            "fields": {}
        },
        "app_update_progress": {
            "kind": "object",
            "fields": {}
        },
        "autoconfigure_groupware": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "background_sync_status": {
            "kind": "object",
            "fields": {}
        },
        "block_sender": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::blocked_senders::BlockSenderInput"
                    }
                }
            }
        },
        "cancel_follow_up": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "cancel_import": {
            "kind": "object",
            "fields": {}
        },
        "cancel_outbox_action": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "cancel_pending_restore": {
            "kind": "object",
            "fields": {}
        },
        "check_app_update": {
            "kind": "object",
            "fields": {}
        },
        "choose_directory": {
            "kind": "object",
            "fields": {
                "defaultPath": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "choose_save_path": {
            "kind": "object",
            "fields": {
                "extensions": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "filterName": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "suggestedName": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "claim_draft_editor": {
            "kind": "object",
            "fields": {
                "draftId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "clear_account_oauth_error": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "complete_oauth_flow": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::oauth::CompleteOAuthInput"
                    }
                }
            }
        },
        "configure_groupware": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::groupware::ConfigureGroupwareInput"
                    }
                }
            }
        },
        "consume_pending_mobile_notification": {
            "kind": "object",
            "fields": {}
        },
        "create_backup": {
            "kind": "object",
            "fields": {
                "acknowledgeUnprotectedKey": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "targetDir": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "create_calendar_event": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::calendar::CalendarEventInput"
                    }
                }
            }
        },
        "create_folder": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::folders::CreateFolderInput"
                    }
                }
            }
        },
        "create_provider_label": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::provider_features::CreateProviderLabelInput"
                    }
                }
            }
        },
        "create_source": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::sources::CreateSourceInput"
                    }
                }
            }
        },
        "credentials_status": {
            "kind": "object",
            "fields": {}
        },
        "current_connection_state": {
            "kind": "object",
            "fields": {}
        },
        "deactivate_account": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_account": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_alias": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_calendar_event": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_contact": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_draft": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_folder": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::folders::DeleteFolderInput"
                    }
                }
            }
        },
        "delete_pgp_key": {
            "kind": "object",
            "fields": {
                "keyId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_rule": {
            "kind": "object",
            "fields": {
                "ruleId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_signature": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_smart_folder": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_smime_certificate": {
            "kind": "object",
            "fields": {
                "certId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_source": {
            "kind": "object",
            "fields": {
                "sourceId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_template": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_topic": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_topic_file": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_topic_note": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "delete_webhook": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "detect_mail_provider": {
            "kind": "object",
            "fields": {
                "domain": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "diagnostics_overview": {
            "kind": "object",
            "fields": {}
        },
        "diagnostics_report": {
            "kind": "object",
            "fields": {}
        },
        "discard_failed_sync_action": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "discover_account": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::onboarding::DiscoverAccountInput"
                    }
                }
            }
        },
        "discover_signature_suggestions": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "identityEmail": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "dismiss_mobile_notifications": {
            "kind": "object",
            "fields": {
                "threadIds": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "download_app_update": {
            "kind": "object",
            "fields": {}
        },
        "download_attachment": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::attachments::DownloadAttachmentInput"
                    }
                }
            }
        },
        "download_topic_file": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "execute_plugin_command": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::plugins::PluginCommandInput"
                    }
                }
            }
        },
        "export_mailbox": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "export_platform_data": {
            "kind": "object",
            "fields": {
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "redactSensitive": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "fetch_server_search_hit": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::server_search::FetchServerHitInput"
                    }
                }
            }
        },
        "focus_draft_editor": {
            "kind": "object",
            "fields": {
                "draftId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "generate_pgp_key": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::pgp::GeneratePgpKeyInput"
                    }
                }
            }
        },
        "generate_smime_keypair": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::smime::GenerateSmimeKeypairInput"
                    }
                }
            }
        },
        "get_message_source": {
            "kind": "object",
            "fields": {
                "messageId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "get_notification_preferences": {
            "kind": "object",
            "fields": {}
        },
        "get_sender_preference": {
            "kind": "object",
            "fields": {
                "senderEmail": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "get_source_capabilities": {
            "kind": "object",
            "fields": {}
        },
        "get_thread": {
            "kind": "object",
            "fields": {
                "threadId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "get_threads": {
            "kind": "object",
            "fields": {
                "threadIds": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "get_topic_timeline": {
            "kind": "object",
            "fields": {
                "topicId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "get_vacation": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "import_ics": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::calendar::ImportIcsInput"
                    }
                }
            }
        },
        "import_ics_path": {
            "kind": "object",
            "fields": {
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "import_mbox": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "import_opml": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::sources::ImportOpmlInput"
                    }
                }
            }
        },
        "import_pgp_keys": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::pgp::ImportPgpKeysInput"
                    }
                }
            }
        },
        "import_platform_data": {
            "kind": "object",
            "fields": {
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "import_smime_pkcs12": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::smime::ImportSmimePkcs12Input"
                    }
                }
            }
        },
        "inline_image_cache_status": {
            "kind": "object",
            "fields": {}
        },
        "install_app_update": {
            "kind": "object",
            "fields": {}
        },
        "list_accounts": {
            "kind": "object",
            "fields": {}
        },
        "list_aliases": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_attachments_for_messages": {
            "kind": "object",
            "fields": {
                "messageIds": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_backups": {
            "kind": "object",
            "fields": {
                "dir": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "list_blocked_senders": {
            "kind": "object",
            "fields": {}
        },
        "list_cached_threads": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::threads::ListThreadsInput"
                        }
                    }
                }
            }
        },
        "list_calendar_events": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_calendar_sources": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_contacts": {
            "kind": "object",
            "fields": {}
        },
        "list_drafts": {
            "kind": "object",
            "fields": {}
        },
        "list_failed_sync_actions": {
            "kind": "object",
            "fields": {}
        },
        "list_follow_ups": {
            "kind": "object",
            "fields": {
                "status": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_invites": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_mailboxes": {
            "kind": "object",
            "fields": {}
        },
        "list_message_bodies": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::messages::MessageBodyIdsInput"
                    }
                }
            }
        },
        "list_messages_preview": {
            "kind": "object",
            "fields": {
                "threadId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "list_open_oauth_flows": {
            "kind": "object",
            "fields": {}
        },
        "list_outbox": {
            "kind": "object",
            "fields": {}
        },
        "list_pgp_keys": {
            "kind": "object",
            "fields": {}
        },
        "list_plugin_audit_log": {
            "kind": "object",
            "fields": {}
        },
        "list_plugin_metrics": {
            "kind": "object",
            "fields": {}
        },
        "list_plugins": {
            "kind": "object",
            "fields": {}
        },
        "list_provider_labels": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_recent_transitions": {
            "kind": "object",
            "fields": {}
        },
        "list_rules": {
            "kind": "object",
            "fields": {}
        },
        "list_signature_assignments": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_signature_suggestions": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_signatures": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_smart_folders": {
            "kind": "object",
            "fields": {}
        },
        "list_smart_inbox_summary": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_smime_certificates": {
            "kind": "object",
            "fields": {}
        },
        "list_sources": {
            "kind": "object",
            "fields": {}
        },
        "list_templates": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "list_threads": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::threads::ListThreadsInput"
                        }
                    }
                }
            }
        },
        "list_topic_files": {
            "kind": "object",
            "fields": {
                "topicId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "list_topic_memberships": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::topics::TopicMembershipsInput"
                    }
                }
            }
        },
        "list_topics": {
            "kind": "object",
            "fields": {}
        },
        "list_webhooks": {
            "kind": "object",
            "fields": {}
        },
        "load_draft": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "load_message_body": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::message_body::LoadMessageBodyInput"
                    }
                }
            }
        },
        "load_thread_messages": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::messages::LoadThreadMessagesInput"
                    }
                }
            }
        },
        "mark_threads_spam": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::thread_actions::SpamThreadsInput"
                    }
                }
            }
        },
        "message_link_target": {
            "kind": "object",
            "fields": {
                "threadId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "mobile_notifications_frontend_ready": {
            "kind": "object",
            "fields": {}
        },
        "move_threads_to_folder": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::move_threads::MoveThreadsInput"
                    }
                }
            }
        },
        "oauth_status": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "open_attachment_default": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::attachments::OpenAttachmentInput"
                    }
                }
            }
        },
        "open_background_sync_settings": {
            "kind": "object",
            "fields": {}
        },
        "open_compose_window": {
            "kind": "object",
            "fields": {
                "draftId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "open_external_url": {
            "kind": "object",
            "fields": {
                "url": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "open_logs_folder": {
            "kind": "object",
            "fields": {}
        },
        "open_message_file": {
            "kind": "object",
            "fields": {
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "open_message_file_in_window": {
            "kind": "object",
            "fields": {
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "open_tray_destination": {
            "kind": "object",
            "fields": {
                "destination": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "open_tray_thread": {
            "kind": "object",
            "fields": {
                "threadId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "pending_crash_reports": {
            "kind": "object",
            "fields": {}
        },
        "pgp_engine_status": {
            "kind": "object",
            "fields": {}
        },
        "poll_oauth_flow": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::oauth::PollOAuthInput"
                    }
                }
            }
        },
        "poll_source": {
            "kind": "object",
            "fields": {
                "sourceId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "prune_backups": {
            "kind": "object",
            "fields": {
                "dir": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "keep": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "read_message_file_attachment": {
            "kind": "object",
            "fields": {
                "index": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "record_frontend_diagnostic": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::diagnostics::FrontendDiagnosticInput"
                    }
                }
            }
        },
        "refresh_provider_labels": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "release_draft_editor": {
            "kind": "object",
            "fields": {
                "draftId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "remove_topic_item": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::topics::RemoveTopicItemInput"
                    }
                }
            }
        },
        "rename_folder": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::folders::RenameFolderInput"
                    }
                }
            }
        },
        "render_template": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::templates::RenderTemplateInput"
                    }
                }
            }
        },
        "resolve_message_link": {
            "kind": "object",
            "fields": {
                "messageId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "resolve_signature": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::signatures::ResolveSignatureInput"
                    }
                }
            }
        },
        "respond_to_invite": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::calendar::InviteResponse"
                    }
                }
            }
        },
        "restore_backup": {
            "kind": "object",
            "fields": {
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "retry_failed_sync_actions": {
            "kind": "object",
            "fields": {}
        },
        "retry_outbox_action": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "run_imap_sync": {
            "kind": "object",
            "fields": {}
        },
        "run_rules_for_thread": {
            "kind": "object",
            "fields": {
                "threadId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "run_rules_for_threads": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::rules::RunRulesForThreadsInput"
                    }
                }
            }
        },
        "runtime_capabilities": {
            "kind": "object",
            "fields": {}
        },
        "save_draft": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "core::drafts::DraftInput"
                    }
                }
            }
        },
        "search_contacts": {
            "kind": "object",
            "fields": {
                "limit": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "query": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "search_message_hits": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "core::db::messages::SearchQuery"
                    }
                }
            }
        },
        "send_message": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "core::drafts::outgoing::OutgoingMessageInput"
                    }
                }
            }
        },
        "server_search": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::server_search::ServerSearchInput"
                    }
                }
            }
        },
        "set_autostart": {
            "kind": "object",
            "fields": {
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "set_background_sync": {
            "kind": "object",
            "fields": {
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "set_contact_avatar": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::contacts::ContactAvatarOverrideInput"
                    }
                }
            }
        },
        "set_notification_preferences": {
            "kind": "object",
            "fields": {
                "preferences": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::notifications::NotificationPreferences"
                    }
                }
            }
        },
        "set_plugin_enabled": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::plugins::PluginEnabledInput"
                    }
                }
            }
        },
        "set_plugin_permission": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::plugins::PluginPermissionInput"
                    }
                }
            }
        },
        "set_quit_on_close": {
            "kind": "object",
            "fields": {
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "set_sender_preference": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::categories::SetSenderPreferenceInput"
                    }
                }
            }
        },
        "set_signature_suggestion_status": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "status": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "set_start_minimized": {
            "kind": "object",
            "fields": {
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "set_thread_category": {
            "kind": "object",
            "fields": {
                "category": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "threadId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "set_thread_muted": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::thread_actions::MuteThreadInput"
                    }
                }
            }
        },
        "set_thread_priority": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::categories::SetThreadPriorityInput"
                    }
                }
            }
        },
        "set_thread_tags": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::threads::SetThreadTagsInput"
                    }
                }
            }
        },
        "set_threads_read": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::threads::UpdateThreadsReadInput"
                    }
                }
            }
        },
        "set_threads_starred": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::star::StarThreadsInput"
                    }
                }
            }
        },
        "set_ui_strings": {
            "kind": "object",
            "fields": {
                "strings": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::ui_strings::UiStrings"
                    }
                }
            }
        },
        "set_vacation": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::vacation::VacationInput"
                    }
                }
            }
        },
        "show_in_folder": {
            "kind": "object",
            "fields": {
                "path": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "start_inline_image_backfill": {
            "kind": "object",
            "fields": {}
        },
        "start_oauth_flow": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::oauth::StartOAuthInput"
                    }
                }
            }
        },
        "stop_inline_image_backfill": {
            "kind": "object",
            "fields": {}
        },
        "sync_contact_avatars": {
            "kind": "object",
            "fields": {
                "allowThirdParty": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "sync_google_calendar": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "sync_groupware": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "sync_local_contact_avatars": {
            "kind": "object",
            "fields": {}
        },
        "take_pending_notification_thread": {
            "kind": "object",
            "fields": {}
        },
        "take_pending_open_with": {
            "kind": "object",
            "fields": {}
        },
        "test_imap_account_connection": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "test_imap_connection": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::sync::TestImapConnectionInput"
                    }
                }
            }
        },
        "toggle_attachment_quick_look": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::attachments::DownloadAttachmentInput"
                    }
                }
            }
        },
        "transition_threads": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "core::workflow::transitions::BatchTransitionInput"
                    }
                }
            }
        },
        "trash_threads": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::threads::ThreadIdsInput"
                    }
                }
            }
        },
        "tray_preferences": {
            "kind": "object",
            "fields": {}
        },
        "tray_snapshot": {
            "kind": "object",
            "fields": {}
        },
        "unblock_sender": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "undo_outbox_send": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "undo_trash": {
            "kind": "object",
            "fields": {
                "receiptId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "unlock_pgp_keys": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::pgp::UnlockPgpKeysInput"
                    }
                }
            }
        },
        "unlock_smime_certificates": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::smime::UnlockSmimeCertificatesInput"
                    }
                }
            }
        },
        "unsubscribe": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::unsubscribe::UnsubscribeInput"
                    }
                }
            }
        },
        "update_account_password": {
            "kind": "object",
            "fields": {
                "accountId": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "password": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "update_thread_organization": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::threads::UpdateThreadOrganizationInput"
                    }
                }
            }
        },
        "upsert_alias": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::aliases::AliasInput"
                    }
                }
            }
        },
        "upsert_contact": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::contacts::ContactInput"
                    }
                }
            }
        },
        "upsert_follow_up": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::followups::FollowUpInput"
                    }
                }
            }
        },
        "upsert_rule": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::rules::UpsertRuleInput"
                    }
                }
            }
        },
        "upsert_signature": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::signatures::SignatureInput"
                    }
                }
            }
        },
        "upsert_signature_assignment": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::signatures::SignatureAssignmentInput"
                    }
                }
            }
        },
        "upsert_smart_folder": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::smart_folders::SmartFolderInput"
                    }
                }
            }
        },
        "upsert_template": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::templates::TemplateInput"
                    }
                }
            }
        },
        "upsert_topic": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::topics::TopicInput"
                    }
                }
            }
        },
        "upsert_topic_note": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::topics::TopicNoteInput"
                    }
                }
            }
        },
        "upsert_webhook": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::webhooks::WebhookInput"
                    }
                }
            }
        },
        "validate_pgp_bundle_passphrase": {
            "kind": "object",
            "fields": {
                "input": {
                    "required": true,
                    "shape": {
                        "kind": "ref",
                        "name": "app::commands::pgp::ValidatePgpBundlePassphraseInput"
                    }
                }
            }
        }
    },
    "types": {
        "app::commands::accounts::NewAccount": {
            "kind": "object",
            "fields": {
                "config": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "display_name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "provider": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::aliases::AliasInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "display_name": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "is_default": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::attachments::DownloadAttachmentInput": {
            "kind": "object",
            "fields": {
                "attachment_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::attachments::OpenAttachmentInput": {
            "kind": "object",
            "fields": {
                "attachment_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "confirmed_executable": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::blocked_senders::BlockSenderInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "pattern": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::calendar::CalendarEventInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "all_day": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "attendees": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "description": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "ends_at": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "location": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "send_invites": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "starts_at": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "title": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::calendar::ImportIcsInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "raw": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "source_filename": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::calendar::InviteResponse": {
            "kind": "object",
            "fields": {
                "comment": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "invite_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "response": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::categories::SetSenderPreferenceInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "category": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "is_priority": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "sender_email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::categories::SetThreadPriorityInput": {
            "kind": "object",
            "fields": {
                "priority": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::contacts::ContactAvatarOverrideInput": {
            "kind": "object",
            "fields": {
                "avatar_url": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "display_name": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "locked": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::contacts::ContactInput": {
            "kind": "object",
            "fields": {
                "avatar_locked": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "avatar_source": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "avatar_url": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "display_name": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "notes": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::diagnostics::FrontendDiagnosticInput": {
            "kind": "object",
            "fields": {
                "body_chars": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "cache_threads": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "cached_messages": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "detail": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "dom_nodes": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "event": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "iframe_count": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "js_heap_bytes": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "level": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "rendered_mail_count": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::folders::CreateFolderInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "parent_path": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::folders::DeleteFolderInput": {
            "kind": "object",
            "fields": {
                "folder_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::folders::RenameFolderInput": {
            "kind": "object",
            "fields": {
                "folder_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "new_name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::followups::FollowUpInput": {
            "kind": "object",
            "fields": {
                "due_at": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "note": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "thread_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::groupware::ConfigureGroupwareInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "caldav_server": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "carddav_server": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "password": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "username": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::message_body::LoadMessageBodyInput": {
            "kind": "object",
            "fields": {
                "message_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::messages::LoadThreadMessagesInput": {
            "kind": "object",
            "fields": {
                "body_count": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::messages::MessageBodyIdsInput": {
            "kind": "object",
            "fields": {
                "message_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::move_threads::MoveThreadsInput": {
            "kind": "object",
            "fields": {
                "target_folder_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::notifications::NotificationPreferences": {
            "kind": "object",
            "fields": {
                "body_window_days": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "scope": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "send_read_receipts": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "sound": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "trash_auto_delete_days": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "undo_send_seconds": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::oauth::AbortOAuthInput": {
            "kind": "object",
            "fields": {
                "state": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::oauth::CompleteOAuthInput": {
            "kind": "object",
            "fields": {
                "code": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "state": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::oauth::PollOAuthInput": {
            "kind": "object",
            "fields": {
                "state": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::oauth::StartOAuthInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "provider": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "purpose": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::onboarding::DiscoverAccountInput": {
            "kind": "object",
            "fields": {
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "imap": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::onboarding::ServerOverride"
                        }
                    }
                },
                "password": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "pop3": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::onboarding::ServerOverride"
                        }
                    }
                },
                "protocol": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "smtp": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::onboarding::ServerOverride"
                        }
                    }
                },
                "username": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::onboarding::ServerOverride": {
            "kind": "object",
            "fields": {
                "host": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "port": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::pgp::GeneratePgpKeyInput": {
            "kind": "object",
            "fields": {
                "algorithm": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "expires_in": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "label": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::pgp::ImportPgpKeysInput": {
            "kind": "object",
            "fields": {
                "armored_bundle": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "label_prefix": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::pgp::UnlockPgpKeysInput": {
            "kind": "object",
            "fields": {
                "key_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::pgp::ValidatePgpBundlePassphraseInput": {
            "kind": "object",
            "fields": {
                "armored_bundle": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "passphrase": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::plugins::PluginCommandInput": {
            "kind": "object",
            "fields": {
                "command": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "payload": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "plugin_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::plugins::PluginEnabledInput": {
            "kind": "object",
            "fields": {
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "plugin_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::plugins::PluginPermissionInput": {
            "kind": "object",
            "fields": {
                "capability": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "granted": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "plugin_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::provider_features::CreateProviderLabelInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "color": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::rules::RunRulesForThreadsInput": {
            "kind": "object",
            "fields": {
                "thread_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::rules::UpsertRuleInput": {
            "kind": "object",
            "fields": {
                "actions": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "description": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "is_active": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "priority": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "trigger_config": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "trigger_type": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::server_search::FetchServerHitInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "remote_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::server_search::ServerSearchInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "query": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::signatures::ResolveSignatureInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "identity_email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "preference": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "signature_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "thread_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::signatures::SignatureAssetInput": {
            "kind": "object",
            "fields": {
                "content_base64": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "content_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "filename": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "mime_type": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::signatures::SignatureAssignmentInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "first_contact_mode": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "identity_email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "signature_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::signatures::SignatureInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "assets": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::signatures::SignatureAssetInput"
                        }
                    }
                },
                "body_html": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "body_text": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "is_default": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "short_body_html": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "short_body_text": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::smart_folders::SmartFolderInput": {
            "kind": "object",
            "fields": {
                "icon": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "query": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "sort_order": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::smime::AddSmimeCertificateInput": {
            "kind": "object",
            "fields": {
                "label": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "pem": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::smime::GenerateSmimeKeypairInput": {
            "kind": "object",
            "fields": {
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "label": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::smime::ImportSmimePkcs12Input": {
            "kind": "object",
            "fields": {
                "label": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "p12_base64": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "passphrase": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::smime::UnlockSmimeCertificatesInput": {
            "kind": "object",
            "fields": {
                "cert_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "passphrase": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::sources::CreateSourceInput": {
            "kind": "object",
            "fields": {
                "color": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "config": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "identity": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "kind": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::sources::ImportOpmlInput": {
            "kind": "object",
            "fields": {
                "notify": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "opml": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "poll_minutes": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::star::StarThreadsInput": {
            "kind": "object",
            "fields": {
                "starred": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::sync::TestImapConnectionInput": {
            "kind": "object",
            "fields": {
                "config": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "email": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::templates::RenderTemplateInput": {
            "kind": "object",
            "fields": {
                "template_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "variables": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::templates::TemplateInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "attachments_json": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "bcc": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "body_html": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "body_text": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "cc": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "shortcut": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "subject": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::thread_actions::MuteThreadInput": {
            "kind": "object",
            "fields": {
                "muted": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::thread_actions::SpamThreadsInput": {
            "kind": "object",
            "fields": {
                "thread_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::threads::ListThreadsInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "before_cursor": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::threads::ThreadCursor"
                        }
                    }
                },
                "contact_email": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "cursor": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "app::commands::threads::ThreadCursor"
                        }
                    }
                },
                "limit": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "mailbox_kind": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "mailbox_value": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "smart_bucket": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "workflow_state": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::threads::SetThreadTagsInput": {
            "kind": "object",
            "fields": {
                "tags": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "thread_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::threads::ThreadCursor": {
            "kind": "object",
            "fields": {
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "last_message_date": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::threads::ThreadIdsInput": {
            "kind": "object",
            "fields": {
                "thread_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::threads::UpdateThreadOrganizationInput": {
            "kind": "object",
            "fields": {
                "assignee": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "board_position": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "shared_with": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "thread_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::threads::UpdateThreadsReadInput": {
            "kind": "object",
            "fields": {
                "is_read": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::topics::AddTopicItemsInput": {
            "kind": "object",
            "fields": {
                "added_by": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "item_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "item_type": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "topic_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::topics::RemoveTopicItemInput": {
            "kind": "object",
            "fields": {
                "item_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "item_type": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "topic_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::topics::TopicFileInput": {
            "kind": "object",
            "fields": {
                "content_base64": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "filename": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "mime_type": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "topic_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::topics::TopicInput": {
            "kind": "object",
            "fields": {
                "color": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "description": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "due_date": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "icon": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "name": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "status": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::topics::TopicMembershipsInput": {
            "kind": "object",
            "fields": {
                "item_ids": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "item_type": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::topics::TopicNoteInput": {
            "kind": "object",
            "fields": {
                "content": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "topic_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::tray::TrayThreadActionInput": {
            "kind": "object",
            "fields": {
                "action": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::ui_strings::UiStrings": {
            "kind": "object",
            "fields": {
                "compose_window_title": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "dock_calendar": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "dock_close_other_windows": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "dock_compose": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "dock_settings": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "dock_sync": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "follow_system_locale": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "lang": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_action_failed_message": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_action_failed_title": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_action_not_applied_message": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_archive": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_background_sync_channel": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_background_sync_description": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_background_sync_text": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_channel_description": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_channel_name": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_follow_up_due": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_new_message": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_no_subject": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_read": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_reply": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_send_failed": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_summary": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_sync_failed": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_syncing": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "notification_trash": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_cancelled_message": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_cancelled_title": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_close_hint": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_connected_message": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_connected_title": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_failed_message": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_failed_title": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_invalid_callback": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_invalid_state": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_invalid_title": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_missing_code": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "oauth_provider_fallback": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "tray_briefing": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "tray_hide": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "tray_quit": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "tray_show": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::unsubscribe::UnsubscribeInput": {
            "kind": "object",
            "fields": {
                "message_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "app::commands::vacation::VacationInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "body": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "ends_at": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "only_contacts": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "starts_at": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "subject": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "app::commands::webhooks::WebhookInput": {
            "kind": "object",
            "fields": {
                "enabled": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "events": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "secret": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "url": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "core::db::messages::SearchCursor": {
            "kind": "object",
            "fields": {
                "date": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "rank_cost": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "core::db::messages::SearchQuery": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "cursor": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "ref",
                            "name": "core::db::messages::SearchCursor"
                        }
                    }
                },
                "limit": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "query": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "core::drafts::DraftInput": {
            "kind": "object",
            "fields": {
                "account_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "attachments": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "ref",
                            "name": "core::drafts::outgoing::OutgoingAttachmentInput"
                        }
                    }
                },
                "bcc": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "body": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "body_html": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "cc": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "expected_revision": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "from_address": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "from_name": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "reply_to": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "scheduled_for": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "send_options": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_override_html": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_override_text": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_variant": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "subject": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "to": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "core::drafts::outgoing::OutgoingAttachmentInput": {
            "kind": "object",
            "fields": {
                "content_base64": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "content_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "filename": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "inline": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "mime_type": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "core::drafts::outgoing::OutgoingMessageInput": {
            "kind": "object",
            "fields": {
                "attachments": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "ref",
                            "name": "core::drafts::outgoing::OutgoingAttachmentInput"
                        }
                    }
                },
                "bcc": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "body": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "body_html": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "cc": {
                    "required": false,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "draft_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "follow_up_at": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "from_address": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "from_name": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "pgp_encrypt_to_recipients": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "pgp_sign_with_key_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "priority": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "reply_to": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "reply_to_message_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "scheduled_for": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_override_html": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_override_text": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "signature_variant": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "smime_encrypt_to_recipients": {
                    "required": false,
                    "shape": {
                        "kind": "value"
                    }
                },
                "smime_sign_with_cert_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "subject": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_id": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "to": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "value"
                        }
                    }
                }
            }
        },
        "core::workflow::transitions::BatchTransitionEntry": {
            "kind": "object",
            "fields": {
                "from": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                },
                "thread_id": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        },
        "core::workflow::transitions::BatchTransitionInput": {
            "kind": "object",
            "fields": {
                "snooze_until": {
                    "required": false,
                    "shape": {
                        "kind": "optional",
                        "item": {
                            "kind": "value"
                        }
                    }
                },
                "threads": {
                    "required": true,
                    "shape": {
                        "kind": "array",
                        "item": {
                            "kind": "ref",
                            "name": "core::workflow::transitions::BatchTransitionEntry"
                        }
                    }
                },
                "to": {
                    "required": true,
                    "shape": {
                        "kind": "value"
                    }
                }
            }
        }
    }
} as const;
