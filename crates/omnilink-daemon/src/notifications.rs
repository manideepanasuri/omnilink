use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use omnilink_protocol::packet::NotificationPacket;
use thiserror::Error;
use zbus::proxy;

#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),
    #[error("Internal synchronization lock error")]
    LockError,
}

#[allow(clippy::too_many_arguments)]
#[proxy(
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    interface = "org.freedesktop.Notifications"
)]
pub trait FreedesktopNotifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: &HashMap<&str, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// Bidirectional map between mobile notification keys and desktop notification IDs.
#[derive(Debug, Default)]
pub struct NotificationMap {
    remote_to_desktop: HashMap<String, u32>,
    desktop_to_remote: HashMap<u32, String>,
}

impl NotificationMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, remote_key: String, desktop_id: u32) {
        if let Some(old_desktop) = self.remote_to_desktop.insert(remote_key.clone(), desktop_id) {
            if old_desktop != desktop_id {
                self.desktop_to_remote.remove(&old_desktop);
            }
        }
        if let Some(old_remote) = self.desktop_to_remote.insert(desktop_id, remote_key) {
            if old_remote != *self.desktop_to_remote.get(&desktop_id).unwrap() {
                self.remote_to_desktop.remove(&old_remote);
            }
        }
    }

    pub fn get_desktop_id(&self, remote_key: &str) -> Option<u32> {
        self.remote_to_desktop.get(remote_key).copied()
    }

    pub fn get_remote_key(&self, desktop_id: u32) -> Option<String> {
        self.desktop_to_remote.get(&desktop_id).cloned()
    }

    pub fn remove_by_remote_key(&mut self, remote_key: &str) -> Option<u32> {
        if let Some(desktop_id) = self.remote_to_desktop.remove(remote_key) {
            self.desktop_to_remote.remove(&desktop_id);
            Some(desktop_id)
        } else {
            None
        }
    }

    pub fn remove_by_desktop_id(&mut self, desktop_id: u32) -> Option<String> {
        if let Some(remote_key) = self.desktop_to_remote.remove(&desktop_id) {
            self.remote_to_desktop.remove(&remote_key);
            Some(remote_key)
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.remote_to_desktop.len()
    }

    pub fn is_empty(&self) -> bool {
        self.remote_to_desktop.is_empty()
    }
}

/// Formats the actions array for org.freedesktop.Notifications.Notify.
///
/// Actions must be formatted as pairs: `[action_key, action_title, ...]`.
/// If `can_reply` is true, an `"inline-reply"` action button is ensured.
pub fn format_notification_actions(packet: &NotificationPacket) -> Vec<String> {
    let mut actions = Vec::new();
    let mut has_inline_reply = false;

    for action in &packet.actions {
        let key = if action.is_input || action.key == "inline-reply" {
            has_inline_reply = true;
            "inline-reply".to_string()
        } else {
            action.key.clone()
        };
        actions.push(key);
        actions.push(action.title.clone());
    }

    if packet.can_reply && !has_inline_reply {
        actions.push("inline-reply".to_string());
        actions.push("Reply".to_string());
    }

    actions
}

/// Forwards mobile notifications to the Linux desktop via org.freedesktop.Notifications D-Bus service.
#[derive(Debug, Clone)]
pub struct NotificationForwarder {
    proxy: FreedesktopNotificationsProxy<'static>,
    map: Arc<RwLock<NotificationMap>>,
}

impl NotificationForwarder {
    pub async fn new() -> Result<Self, NotificationError> {
        let connection = zbus::Connection::session().await?;
        let proxy = FreedesktopNotificationsProxy::new(&connection).await?;
        Ok(Self {
            proxy,
            map: Arc::new(RwLock::new(NotificationMap::new())),
        })
    }

    pub fn from_proxy(proxy: FreedesktopNotificationsProxy<'static>) -> Self {
        Self {
            proxy,
            map: Arc::new(RwLock::new(NotificationMap::new())),
        }
    }

    pub fn proxy(&self) -> &FreedesktopNotificationsProxy<'static> {
        &self.proxy
    }

    pub fn map(&self) -> Arc<RwLock<NotificationMap>> {
        self.map.clone()
    }

    pub fn get_desktop_id(&self, remote_key: &str) -> Option<u32> {
        self.map.read().ok()?.get_desktop_id(remote_key)
    }

    pub fn get_remote_key(&self, desktop_id: u32) -> Option<String> {
        self.map.read().ok()?.get_remote_key(desktop_id)
    }

    pub fn remove_by_remote_key(&self, remote_key: &str) -> Option<u32> {
        self.map.write().ok()?.remove_by_remote_key(remote_key)
    }

    pub fn remove_by_desktop_id(&self, desktop_id: u32) -> Option<String> {
        self.map.write().ok()?.remove_by_desktop_id(desktop_id)
    }

    pub async fn forward_notification(
        &self,
        packet: &NotificationPacket,
    ) -> Result<u32, NotificationError> {
        let replaces_id = self
            .map
            .read()
            .map_err(|_| NotificationError::LockError)?
            .get_desktop_id(&packet.id)
            .unwrap_or(0);

        let actions = format_notification_actions(packet);
        let action_refs: Vec<&str> = actions.iter().map(AsRef::as_ref).collect();
        let hints = HashMap::new();

        let desktop_id = self
            .proxy
            .notify(
                &packet.app_name,
                replaces_id,
                "",
                &packet.title,
                &packet.text,
                &action_refs,
                &hints,
                -1,
            )
            .await?;

        self.map
            .write()
            .map_err(|_| NotificationError::LockError)?
            .insert(packet.id.clone(), desktop_id);

        Ok(desktop_id)
    }

    pub async fn close_desktop_notification(
        &self,
        remote_key: &str,
    ) -> Result<bool, NotificationError> {
        let desktop_id = match self.remove_by_remote_key(remote_key) {
            Some(id) => id,
            None => return Ok(false),
        };

        match self.proxy.close_notification(desktop_id).await {
            Ok(()) => Ok(true),
            Err(e) => {
                tracing::warn!(desktop_id, error = %e, "Failed to close desktop notification");
                Err(NotificationError::Dbus(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnilink_protocol::packet::NotificationAction;

    #[test]
    fn test_notification_map_insert_and_lookup() {
        let mut map = NotificationMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);

        map.insert("0|com.whatsapp|1234".to_string(), 101);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get_desktop_id("0|com.whatsapp|1234"), Some(101));
        assert_eq!(map.get_remote_key(101), Some("0|com.whatsapp|1234".to_string()));
        assert_eq!(map.get_desktop_id("non-existent"), None);
        assert_eq!(map.get_remote_key(999), None);
    }

    #[test]
    fn test_notification_map_replaces_and_consistency() {
        let mut map = NotificationMap::new();
        map.insert("key1".to_string(), 100);
        assert_eq!(map.get_desktop_id("key1"), Some(100));

        // Replaces key1 with new desktop_id 101
        map.insert("key1".to_string(), 101);
        assert_eq!(map.get_desktop_id("key1"), Some(101));
        assert_eq!(map.get_remote_key(100), None);
        assert_eq!(map.get_remote_key(101), Some("key1".to_string()));

        // Mapping desktop_id 101 to key2 evicts key1 from reverse
        map.insert("key2".to_string(), 101);
        assert_eq!(map.get_desktop_id("key2"), Some(101));
        assert_eq!(map.get_remote_key(101), Some("key2".to_string()));
        assert_eq!(map.get_desktop_id("key1"), None);
    }

    #[test]
    fn test_notification_map_removals() {
        let mut map = NotificationMap::new();
        map.insert("key1".to_string(), 10);
        map.insert("key2".to_string(), 20);

        assert_eq!(map.remove_by_remote_key("key1"), Some(10));
        assert_eq!(map.get_desktop_id("key1"), None);
        assert_eq!(map.get_remote_key(10), None);
        assert_eq!(map.remove_by_remote_key("key1"), None);

        assert_eq!(map.remove_by_desktop_id(20), Some("key2".to_string()));
        assert_eq!(map.get_desktop_id("key2"), None);
        assert_eq!(map.get_remote_key(20), None);
        assert_eq!(map.remove_by_desktop_id(20), None);
        assert!(map.is_empty());
    }

    #[test]
    fn test_format_notification_actions() {
        let packet_no_reply = NotificationPacket {
            id: "1".to_string(),
            package_name: "pkg".to_string(),
            app_name: "App".to_string(),
            title: "Title".to_string(),
            text: "Text".to_string(),
            icon_png: None,
            timestamp: 100,
            can_reply: false,
            actions: vec![NotificationAction {
                key: "dismiss".to_string(),
                title: "Dismiss".to_string(),
                is_input: false,
            }],
        };

        let actions = format_notification_actions(&packet_no_reply);
        assert_eq!(actions, vec!["dismiss", "Dismiss"]);

        let packet_with_reply = NotificationPacket {
            id: "2".to_string(),
            package_name: "pkg".to_string(),
            app_name: "App".to_string(),
            title: "Title".to_string(),
            text: "Text".to_string(),
            icon_png: None,
            timestamp: 100,
            can_reply: true,
            actions: vec![NotificationAction {
                key: "open".to_string(),
                title: "Open".to_string(),
                is_input: false,
            }],
        };

        let actions = format_notification_actions(&packet_with_reply);
        assert_eq!(actions, vec!["open", "Open", "inline-reply", "Reply"]);

        let packet_with_input_action = NotificationPacket {
            id: "3".to_string(),
            package_name: "pkg".to_string(),
            app_name: "App".to_string(),
            title: "Title".to_string(),
            text: "Text".to_string(),
            icon_png: None,
            timestamp: 100,
            can_reply: true,
            actions: vec![NotificationAction {
                key: "custom_reply_key".to_string(),
                title: "Quick Reply".to_string(),
                is_input: true,
            }],
        };

        let actions = format_notification_actions(&packet_with_input_action);
        assert_eq!(actions, vec!["inline-reply", "Quick Reply"]);
    }

    #[tokio::test]
    async fn test_dbus_session_forwarder_lifecycle_if_available() {
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
            eprintln!("Skipping live D-Bus test: DBUS_SESSION_BUS_ADDRESS not configured");
            return;
        }

        let forwarder = match NotificationForwarder::new().await {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Skipping live D-Bus test: session connection failed: {e}");
                return;
            }
        };

        let packet = NotificationPacket {
            id: "omnilink-test-key-001".to_string(),
            package_name: "org.omnilink.test".to_string(),
            app_name: "OmniLink Test".to_string(),
            title: "Test Notification".to_string(),
            text: "Testing D-Bus forwarder".to_string(),
            icon_png: None,
            timestamp: 1729250000000,
            can_reply: true,
            actions: vec![],
        };

        // Forward to desktop
        let desktop_id = forwarder.forward_notification(&packet).await.expect("Failed to forward notification");
        assert!(desktop_id > 0);
        assert_eq!(forwarder.get_desktop_id(&packet.id), Some(desktop_id));
        assert_eq!(forwarder.get_remote_key(desktop_id), Some(packet.id.clone()));

        // Immediately close to avoid leaving test popup
        let closed = forwarder.close_desktop_notification(&packet.id).await.expect("Failed to close notification");
        assert!(closed);
        assert_eq!(forwarder.get_desktop_id(&packet.id), None);
    }
}
