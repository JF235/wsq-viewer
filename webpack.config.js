//@ts-check

'use strict';

const path = require('path');
const CopyPlugin = require('copy-webpack-plugin');

/**@type {import('webpack').Configuration}*/
const config = {
  target: 'node', 
  mode: 'none', 

  entry: './src/extension.ts', 
  output: {
    // Verifique se o path é 'dist' ou 'out' no seu package.json
    path: path.resolve(__dirname, 'dist'), 
    filename: 'extension.js',
    libraryTarget: 'commonjs2'
  },
  externals: {
    vscode: 'commonjs vscode' 
  },
  resolve: {
    extensions: ['.ts', '.js']
  },
  module: {
    rules: [
      {
        test: /\.ts$/,
        exclude: /node_modules/,
        use: [
          {
            loader: 'ts-loader'
          }
        ]
      }
    ]
  },
  plugins: [
    new CopyPlugin({
      patterns: [
        // Copia o script Python
        { from: 'src/decoder.py', to: '.' },
        // NOVO: Copia a pasta 'media' inteira
        { from: 'media', to: 'media' } 
      ],
    }),
  ],
  devtool: 'nosources-source-map',
  infrastructureLogging: {
    level: "log",
  },
};
module.exports = config;

