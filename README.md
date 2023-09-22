# sifis-message
[![Actions Status][actions badge]][actions]
[![CodeCov][codecov badge]][codecov]
[![LICENSE][license badge]][license]

[SIFIS-Home](https://sifis-home.eu) developer API runtime and WoT consumer through DHT messages



![message-diagram](assets/sifis-message.drawio.svg)

## Programs

### `sifis-runtime`

It is a SIFIS-Home developer API runtime implementation that uses the DHT as a mean to interact
with the [usage-control](https://github.com/sifis-home/usage-control) system to control SIFIS-Home
Web of Things NSSDs via a distributed consumer.

### `sifis-consumer`

Reference implementation of a Web of Things consumer interacting with the SIFIS-Home developer API
runtime.

[actions]: https://github.com/sifis-home/sifis-message/actions
[codecov]: https://codecov.io/gh/sifis-home/sifis-message
[license]: LICENSES/MIT.txt
[actions badge]: https://github.com/sifis-home/sifis-message/workflows/sifis-message/badge.svg
[codecov badge]: https://codecov.io/gh/sifis-home/sifis-message/branch/master/graph/badge.svg
[license badge]: https://img.shields.io/badge/license-MIT-blue.svg
