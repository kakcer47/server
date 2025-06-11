const express = require('express');
const multer = require('multer');
const fetch = require('node-fetch');
const compression = require('compression');
const cors = require('cors');
const app = express();
const port = process.env.PORT || 3000;

// Оптимизация: минимизация задержек
app.use(compression()); // Сжатие ответов
app.use(cors()); // Разрешить CORS для фронтенда
app.use(express.json({ limit: '5mb' })); // Ограничение размера тела запроса
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

        // Обработка текста (отправка в JSON Blob)
    if (req.body.text) {
        const textResponse = await fetch('https://jsonblob.com/api/jsonBlob', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ content: req.body.text })
        });

        // Извлечение ID из заголовка Location
        if (textResponse.ok) {
            const locationHeader = textResponse.headers.get('Location');
            if (locationHeader) {
                // Формируем URL для доступа к JSON Blob
                const blobId = locationHeader.split('/').pop();
                textUrl = `https://jsonblob.com/api/jsonBlob/${blobId}`;
            } else {
                throw new Error('No Location header in JSON Blob response');
            }
        } else {
            throw new Error('Failed to upload text to JSON Blob');
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
            const imageResult = await imageResponse.json();

            if (imageResult.success) {
                imageUrl = imageResult.url;
            } else {
                throw new Error('Failed to upload image to Postimg.cc');
            }
        }

        res.json({
            success: true,
            textUrl: textUrl,
            imageUrl: imageUrl
        });
    } catch (error) {
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
