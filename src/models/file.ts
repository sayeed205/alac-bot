import * as mongoose from 'mongoose';

import { FileTypeEnum } from '../types';

const fileSchema = new mongoose.Schema(
    {
        appleId: { type: Number, required: true },
        type: {
            type: String,
            enum: Object.values(FileTypeEnum),
            required: true,
        },
        tgFileId: [
            {
                _id: false,
                type: String,
                required: true,
            },
        ],
    },
    {
        methods: {},
        timestamps: true,
        statics: {
            async addFile(
                appleId: number,
                type: FileTypeEnum,
                tgFileId: string
            ) {
                return await this.create({
                    appleId,
                    type,
                    tgFileId: [tgFileId],
                });
            },
        },
    }
);

export type FILE = mongoose.InferSchemaType<typeof fileSchema>;
export const File = mongoose.model('file', fileSchema);
