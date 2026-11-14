---
"architect-app" = "minor"
"architect-extensions" = "minor"
"architect-mobile" = "minor"
---

Added the device's services for extensions (#709): `capabilities.device` (`notifications`, `camera`, `photos`, `health:read:<type>`, `health:write:<type>`, consented and granted entry by entry) and the host calls `os_notify`, `cancel_os_notify`, `capture`, `photos`, `photo_file`, `health_read`, `health_write`, answered by the platform's `device::Device` (`Host::set_device`; `NoDevice`, `FakeDevice`): on iOS `device_ios::IosDevice` (feature `device`: UserNotifications, UIImagePickerController, Photos, HealthKit), elsewhere `unsupported`; the iOS app's Info.plist gains the camera, photo library and health usage strings
