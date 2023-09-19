# sifis-message

[SIFIS-Home](https://sifis-home.eu) developer API runtime and WoT consumer through DHT messages
[![Actions Status][actions badge]][actions]
[![CodeCov][codecov badge]][codecov]
[![LICENSE][license badge]][license]

![message-diagram](assets/sifis-message.drawio.svg)

## Programs

### `sifis-runtime`

It is a SIFIS-Home developer API runtime implementation that uses the DHT as a mean to interact
with the [usage-control](https://github.com/sifis-home/usage-control) system to control SIFIS-Home
Web of Things NSSDs via a distributed consumer.

### `sifis-consumer`

Reference implementation of a Web of Things consumer interacting with the SIFIS-Home developer API
runtime.

[actions]: https://github.com/<your-account>/sifis-message/actions
[codecov]: https://codecov.io/gh/<your-account>/sifis-message
[license]: LICENSES/MIT.txt
[actions badge]: https://github.com/<your-account>/sifis-message/workflows/sifis-message/badge.svg
[codecov badge]: https://codecov.io/gh/<your-account>/sifis-message/branch/master/graph/badge.svg
[license badge]: https://img.shields.io/badge/license-MIT-blue.svg
