#include "tra86.h"
#include "./ui_tra86.h"
#include <QFileDialog>
#include <QMessageBox>

tra86::tra86(QWidget *parent)
    : QMainWindow(parent)
    , ui(new Ui::tra86)
{
    ui->setupUi(this);
}

tra86::~tra86()
{
    delete ui;
}

void tra86::newDocument() {
    setWindowTitle("tra86");
    currentFile.clear();
    ui->textEdit->setText(QString());
}

void tra86::on_actionOpen_triggered()
{
    QString fileName = QFileDialog::getOpenFileName(this, "Open the file");
    if (fileName.isEmpty())
        return;
    QFile file(fileName);
    currentFile = fileName;
    if (!file.open(QIODevice::ReadOnly | QFile::Text)) {
        QMessageBox::warning(this, "Warning", "Cannot open file: " + file.errorString());
        return;
    }
    setWindowTitle(fileName + " | tra86");
    QTextStream in(&file);
    QString text = in.readAll();
    ui->textEdit->setText(text);
    file.close();
}


void tra86::on_actionSave_triggered()
{
    QString fileName;
    //If we don't have a filename from before, get a new fileName
    if(currentFile.isEmpty()) {
        fileName = QFileDialog::getSaveFileName(this, "Save");
        if(fileName.isEmpty()) {
            return;
        }
        else {
            fileName = currentFile;
        }
    }
    //Instantiate new file with this new file name
    QFile file(fileName);
    if (!file.open(QIODevice::WriteOnly | QFile::Text)) {
        QMessageBox::warning(this, "Warning", "Cannot save file: " + file.errorString());
        return;
    }
    setWindowTitle(fileName + " | tra86");
    QTextStream out(&file);
    QString text = ui->textEdit->toPlainText();
    out << text;
    file.close();
}


void tra86::on_actionNew_triggered()
{
    newDocument();
}

