import * as mongoose from 'mongoose';

const chatSchema = new mongoose.Schema(
    {
        chatId: { type: Number, required: true },
    },
    {
        methods: {},
        timestamps: true,
        statics: {
            async authorize(chatId: number) {
                const chat = await this.findOne({ chatId });
                if (chat) return chat;
                return await this.create({ chatId });
            },
        },
    }
);

export type CHAT = mongoose.InferSchemaType<typeof chatSchema>;
export const Chat = mongoose.model('chat', chatSchema);
