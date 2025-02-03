import process from 'node:process';

const API_ID = Number.parseInt(process.env.API_ID!);
const API_HASH = process.env.API_HASH!;
const BOT_TOKEN = process.env.BOT_TOKEN!;
const MONGO_URI = process.env.MONGO_URI!;
const ADMIN_ID = process.env.ADMIN_ID!;
const DUMP_ID = process.env.DUMP_ID!;

if (Number.isNaN(API_ID) || !API_HASH) {
    throw new Error('API_ID or API_HASH not set!');
}

export { ADMIN_ID, API_HASH, API_ID, BOT_TOKEN, DUMP_ID, MONGO_URI };
