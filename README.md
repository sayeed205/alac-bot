# alac-bun

[mtcute](https://mtcute.dev/) powered Telegram bot to rip Apple Music songs in ALAC format.

Go binary is needed to build the shared library for the apple music ripper which is written in go and the bot logic is written in bun/ts using [mtcute](https://mtcute.dev/)

```bash
go build -o wrapper.<suffix> -buildmode=c-shared wrapper.go # suffix is .so for linux, .dylib for macos, .dll for windows
```

## Running the Bot

```bash
bun i
bun i # twice because `ref` gives errors
cp .env.example .env
# edit .env
bun dev # or bun start
```

## Environment Variables

-   `API_ID` and `API_HASH` - Telegram API credentials
-   `BOT_TOKEN` - Bot token
-   `MONGO_URI` - MongoDB URI
-   `ADMIN_ID` - Admin ID (your telegram ID)
-   `DUMP_ID` - Chat ID where the bot will dump the ripped songs for future use
-   `M3U8_URL` - URL of the device checkout - [wrapper](https://github.com/zhaarey/wrapper)
-   `DEC_URL` - URL of the decryptor checkout - [wrapper](https://github.com/zhaarey/wrapper)

## Commands

-   `/start` - Start the bot
-   `/song` - Reply to an Apple Music link to rip the song
-   `/help` - Get help
-   `/authorize` - Authorize the bot to a chat or group or user
-   `/id` - Get the chat ID
-   `/ping` - Check if the bot is alive - pong

## Credits

-   [mtcute](https://mtcute.dev/) - Telegram client library
-   [wrapper](https://github.com/zhaarey/wrapper) - Apple Music ripper
-   [bun](https://bun.sh) - JS/TS Runtime
-   [mongodb](https://mongodb.com) - Database

## License

[MIT](./LICENSE)

## Disclaimer

This bot is for educational purposes only. I am not responsible for any misuse of this bot. I do not support piracy. Use this bot at your own risk. I am not responsible for any legal issues that may arise from the use of this bot. I am not anyway affiliated with Apple Music or Telegram.
