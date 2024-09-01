#ifndef TRA86_H
#define TRA86_H

#include <iostream>
#include <QMainWindow>

QT_BEGIN_NAMESPACE
namespace Ui {
class tra86;
}
QT_END_NAMESPACE

class tra86 : public QMainWindow
{
    Q_OBJECT

public:
    tra86(QWidget *parent = nullptr);
    ~tra86();
    void newDocument();

private slots:
    void on_actionOpen_triggered();

    void on_actionSave_triggered();

    void on_actionNew_triggered();

    void on_actionC_3_triggered();

private:
    Ui::tra86 *ui;
    QString currentFile;
};
#endif // TRA86_H
