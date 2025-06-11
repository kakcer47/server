const express = require('express');
const multer = require('multer');
const fetch = require('node-fetch');
const compression = require('compression');
const cors = require('cors');
const FormData = require('form-data');
const app = express();
const port = process.env.PORT || 3000;

// Оптимизация: минимизация задержек
app.use(compression());
app.use(cors());
app.use(express.json({ limit: '5mb' }));
app.use(express.urlencoded({ extended: true }));

// Настройка multer для обработки файлов
const upload = multer({
    limits: { fileSize: 5 * 1024 * 1024 }, // 5 МБ лимит
    storage: multer.memoryStorage()
});

// Эндпоинт для загрузки текста и изображения
app.post('/upload', upload.single('image'), async (req, res) => {
    try {
        let textUrl = '';
        let imageUrl = '';

        // Логирование входящего запроса
        console.log('Received request:', {
            hasText: !!req.body.text,
            hasImage: !!req.file
        });

        // Обработка текста (отправка в JSON Blob)
        if (req.body.text) {
            const textResponse = await fetch('https://jsonblob.com/api/jsonBlob', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ content: req.body.text })
            });

            const textBody = await textResponse.text();
            console.log('JSON Blob response:', {
                status: textResponse.status,
                body: textBody
            });

            if (textResponse.ok) {
                const locationHeader = textResponse.headers.get('Location');
                if (locationHeader) {
                    const blobId = locationHeader.split('/').pop();
                    textUrl = `https://jsonblob.com/api/jsonBlob/${blobId}`;
                } else {
                    throw new Error('No Location header in JSON Blob response');
                }
            } else {
                throw new Error(`JSON Blob failed: ${textBody}`);
            }
        }

        // Обработка изображения (отправка в Postimg.cc)
        if (req.file) {
            const formData = new FormData();
            formData.append('upload', req.file.buffer, req.file.originalname);

            const imageResponse = await fetch('https://postimg.cc/upload', {
                method: 'POST',
                body: formData
            });

            const imageBody = await imageResponse.text();
            console.log('Postimg.cc response:', {
                status: imageResponse.status,
                body: imageBody
            });

            let imageResult;
            try {
                imageResult = JSON.parse(imageBody);
            } catch (parseError) {
                throw new Error(`Invalid JSON from Postimg.cc: ${imageBody}`);
            }

            if (imageResult.success) {
                imageUrl = imageResult.url;
            } else {
                throw new Error(`Postimg.cc upload failed: ${imageBody}`);
            }
        }

        // Логирование успешного ответа
        console.log('Sending response:', { textUrl, imageUrl });

        res.setHeader('Content-Type', 'application/json');
        res.json({
            success: true,
            textUrl: textUrl,
            imageUrl: imageUrl
        });
    } catch (error) {
        // Логирование ошибки
        console.error('Error in /upload:', error.message);
        res.status(500).json({
            success: false,
            error: error.message
        });
    }
});

// Запуск сервера
app.listen(port, () => {
    console.log(`Server running on port ${port}`);
});
