# ALAC Bot

## Project Setup

To set up the Apple Bot project, follow these steps:

1. **Clone the Repository**:

    ```bash
    git clone https://github.com/sayeed205/alac-bot.git
    cd alac-bot
    ```

2. **Install Dependencies**:
   Ensure you have Go installed on your machine. You can install the required dependencies using:

    ```bash
    go mod tidy
    ```

3. **Environment Variables**:
   Create a `.env` file in the root directory of the project and set the following environment variables:
    - `ADMIN_ID`: Your Telegram UserID
    - `BOT_TOKEN`: OmaeWaMouShindeiru
    - `MONGO_URL`: The connection string for your MongoDB database.(default db "alac-bot", hard coded hui hui)
    - `MEDIA_USER_TOKEN`: Needed if you want to embed lyrics (not implemented yet)
    - `DEVICE_URL`: 127.0.0.1:20020 check out [guide](https://github.com/zhaarey/wrapper)
    - `DECRYPTION_URL`: 127.0.0.1:10020 check out [guide](https://github.com/zhaarey/wrapper)

## Running the Bot

To run the bot, execute the following command in your terminal:
   ```bash
   go run .
   ```