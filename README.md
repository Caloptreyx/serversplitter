# Server Splitter

A Calagopus Panel extension (`com.caloptreyx.serversplitter`, requires panel `>=1.1.0`) that lets users split a master server's resources into separate child servers, each with its own console, files and egg. A **Splitter** tab on the server carves CPU, memory, disk and feature limits (allocations, databases, backups, schedules) out of the master's pool, and deleting a split returns them. Admins configure reserved master limits, the default split limit and egg rules (which child eggs may be used for which parent eggs) from the extension's configuration page, and can set a per-server splits limit on the server forms.

## Support

Need help or want to request a feature? Join the [Caloptreyx Discord](https://discord.gg/4qjMWU7S8x).

## License

Released under the MIT License. See [LICENSE](LICENSE).
