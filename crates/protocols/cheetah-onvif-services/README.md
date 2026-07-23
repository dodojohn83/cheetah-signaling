# cheetah-onvif-services

Sans-I/O ONVIF service request builders, response parsers, and wire types.

This crate sits at the protocol core/foundation layer and is shared by
`cheetah-onvif-module` (protocol module) and `cheetah-onvif-driver-tokio`
(protocol driver). It must not contain tokio, HTTP client, or application
logic.
