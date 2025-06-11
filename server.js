const express = require('express');
const fetch = require('node-fetch');
const compression = require('compression');
const cors = require('cors');
const app = express();
const port = process.env.PORT || 3000;

app.use(compression());
app.use(cors());
app.use(express.json({ limit: '5mb' }));
app.use(express.urlencoded({ extended: true }));

// Эндпоинт для загрузки текста
app.post('/upload-text', async (req, res) => {
    try {
        let textUrl = '';

        console.log('Received request:', { hasText: !!req.body.text });

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

        console.log('Sending response:', { textUrl });

        res.setHeader('Content-Type', 'application/json');
        res.json({
            success: true,
            textUrl: textUrl
        });
    } catch (error) {
        console.error('Error in /upload-text:', error.message);
        res.status(500).json({
            success: false,
            error: error.message
        });
    }
});

app.listen(port, () => {
    console.log(`Server running on port ${port}`);
});
