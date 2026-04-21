const fs = require('fs');
const data = fs.readFileSync('config.json', 'utf-8');
console.log(JSON.parse(data));
