# cheetah-onvif-module

ONVIF protocol module placeholder. The full Device/Discovery/Provisioning
implementation lives in the `devin/phase-17-onvif-services` branch and will be
merged into this crate once the plugin host wiring is in place.

Public entry point: [`driver`](src/driver.rs) exposes `OnvifDriverFactory` and
`OnvifProtocolDriver` for the shared plugin SDK port.
